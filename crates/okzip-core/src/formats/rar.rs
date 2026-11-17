//! RAR backend (list/extract/test only) via the pure-Rust `rars` crate.
//!
//! Supports RAR 1.3 through RAR 7 archives, solid archives, AES-encrypted
//! files and headers, and multi-volume sets (`x.part1.rar` and `x.rar` +
//! `x.r00`...). Extraction streams into our own writers, so progress is
//! reported per byte and cancellation is checked on every write.

use crate::error::{cancelled_io, Error, Result};
use crate::progress::Progress;
use crate::types::{ArchiveFormat, ArchiveInfo, Entry, ExtractOptions, OpStats, OpenOptions, TestReport};
use crate::util::{
    decode_legacy_name, dos_to_unix, normalize_entry_path, set_mtime, system_time_to_unix, OutputPlanner, Planned,
};
use rars::{Archive, ArchiveFamily, ArchiveMember, ArchiveMemberDetail, ArchiveReadOptions, ExtractedEntryMeta};
use rars::{ExtractionDecision, ExtractionErrorAction};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Volumes

/// `name.part07.rar` → (`name`, 2 digits... width of the number, number).
fn part_naming(file_name: &str) -> Option<(String, usize)> {
    let lower = file_name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".rar")?;
    let i = stem.rfind(".part")?;
    let digits = &stem[i + 5..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((file_name[..i].to_string(), digits.len()))
}

/// Every physical file that belongs to the RAR set `path` is part of, in
/// order. Starts at the first volume when it exists next to `path`.
/// `is_volume` is the archive's own volume flag (old-style `.rNN` sets are
/// only probed when it is set, so a lone `x.rar` never picks up strangers).
pub(crate) fn volume_paths(path: &Path, is_volume: bool) -> Vec<PathBuf> {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if let Some((base, width)) = part_naming(&name) {
        let ext = &name[name.len() - 4..]; // keep ".rar"/".RAR" casing
        let mut vols = Vec::new();
        for n in 1..10_000u32 {
            let p = dir.join(format!("{base}.part{n:0width$}{ext}"));
            if p.is_file() {
                vols.push(p);
            } else {
                break;
            }
        }
        if !vols.is_empty() {
            return vols;
        }
        return vec![path.to_path_buf()];
    }
    if !is_volume {
        return vec![path.to_path_buf()];
    }
    // Old naming: x.rar, x.r00 .. x.r99, x.s00 .. (opened from any of them).
    let lower = name.to_ascii_lowercase();
    let Some(dot) = lower.rfind('.') else { return vec![path.to_path_buf()] };
    let ext = &lower[dot + 1..];
    let old_style = ext == "rar"
        || (ext.len() == 3 && ext.as_bytes()[0].is_ascii_lowercase() && ext[1..].bytes().all(|b| b.is_ascii_digit()));
    if !old_style {
        return vec![path.to_path_buf()];
    }
    let base = &name[..dot];
    let upper = name[dot + 1..].chars().next().is_some_and(|c| c.is_ascii_uppercase());
    let ext_of = |s: &str| if upper { s.to_ascii_uppercase() } else { s.to_string() };
    let first = dir.join(format!("{base}.{}", ext_of("rar")));
    if !first.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut vols = vec![first];
    'outer: for letter in b'r'..=b'z' {
        for n in 0..100u32 {
            let p = dir.join(format!("{base}.{}", ext_of(&format!("{}{n:02}", letter as char))));
            if p.is_file() {
                vols.push(p);
            } else {
                break 'outer;
            }
        }
    }
    vols
}

fn is_volume(a: &Archive) -> bool {
    a.as_rar15_40().is_some_and(|a| a.main.is_volume()) || a.as_rar50().is_some_and(|a| a.main.is_volume())
}

fn has_encrypted_headers(a: &Archive) -> bool {
    a.as_rar15_40().is_some_and(|a| a.main.has_encrypted_headers()) || a.as_rar50().is_some_and(|a| a.main.encrypted_headers)
}

fn is_solid(a: &Archive) -> bool {
    if let Some(a) = a.as_rar13() {
        return a.main.is_solid();
    }
    if let Some(a) = a.as_rar15_40() {
        return a.main.is_solid();
    }
    a.as_rar50().is_some_and(|a| a.main.is_solid())
}

/// See `walk`. rars' own default is 512 MiB.
const RAR5_BUFFERED_LIMIT: u64 = 16 * 1024 * 1024;

fn pw_bytes(pw: Option<&str>) -> Option<&[u8]> {
    pw.filter(|s| !s.is_empty()).map(str::as_bytes)
}

/// A parsed RAR set: one `rars::Archive` per physical volume.
struct RarSet {
    vols: Vec<Archive>,
    paths: Vec<PathBuf>,
}

impl RarSet {
    fn open(path: &Path, pw: Option<&str>) -> Result<RarSet> {
        let read = |p: &Path| -> Result<Archive> {
            rars::ArchiveReader::read_path_with_options(p, ArchiveReadOptions::with_optional_password(pw_bytes(pw)))
                .map_err(|e| {
                    let mapped = map_err(e, pw);
                    // RAR 3/4 encrypted headers have no password check: a wrong
                    // password just yields garbage headers. If the archive needs
                    // a password at all, blame the password.
                    if pw_bytes(pw).is_some() && matches!(mapped, Error::Corrupt(_) | Error::Other(_)) {
                        let without = rars::ArchiveReader::read_path_with_options(p, ArchiveReadOptions::new());
                        if without.is_err_and(|e| e.kind() == rars::ErrorKind::PasswordRequired) {
                            return Error::WrongPassword;
                        }
                    }
                    mapped
                })
        };
        let first = read(path)?;
        let paths = volume_paths(path, is_volume(&first));
        if paths.len() <= 1 {
            return Ok(RarSet { vols: vec![first], paths: vec![path.to_path_buf()] });
        }
        let mut vols = Vec::with_capacity(paths.len());
        for p in &paths {
            vols.push(if p == path { first.clone() } else { read(p)? });
        }
        Ok(RarSet { vols, paths })
    }

    fn family(&self) -> ArchiveFamily {
        self.vols[0].family()
    }

    fn members(&self) -> Result<Vec<ArchiveMember>> {
        if self.vols.len() == 1 {
            return Ok(self.vols[0].members().collect());
        }
        // Like `rars::volume_members`, but the RAR 3/4 file CRC of a split
        // member is only the whole-file CRC in its *last* fragment (earlier
        // fragments carry the CRC of their own packed piece), so keep that.
        let mut out: Vec<ArchiveMember> = Vec::new();
        for vol in &self.vols {
            for m in vol.members() {
                if !m.meta.is_split_before {
                    out.push(m);
                    continue;
                }
                let Some(prev) = out.last_mut() else {
                    return Err(Error::Corrupt("volume set starts with a continuation".into()));
                };
                prev.meta.packed_size = prev.meta.packed_size.saturating_add(m.meta.packed_size);
                prev.meta.is_split_after = m.meta.is_split_after;
                if let (
                    ArchiveMemberDetail::Rar15To40 { crc32: dst, .. },
                    ArchiveMemberDetail::Rar15To40 { crc32: src, .. },
                ) = (&mut prev.detail, &m.detail)
                {
                    *dst = *src;
                }
            }
        }
        Ok(out)
    }

    fn total_size(&self) -> u64 {
        self.paths.iter().map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum()
    }
}

// ---------------------------------------------------------------------------
// Metadata

fn decode_name(raw: &[u8], unicode: bool, family: ArchiveFamily) -> String {
    let text = if family == ArchiveFamily::Rar50Plus {
        String::from_utf8_lossy(&rars::filename::decode_rar50(raw)).into_owned()
    } else if unicode {
        String::from_utf8_lossy(raw).into_owned()
    } else {
        decode_legacy_name(raw)
    };
    normalize_entry_path(&text.replace('\\', "/"))
}

fn raw_time_to_unix(raw: Option<u32>, family: ArchiveFamily) -> Option<i64> {
    let raw = raw?;
    if family == ArchiveFamily::Rar50Plus {
        Some(i64::from(raw))
    } else {
        dos_to_unix(raw)
    }
}

fn member_to_entry(m: &ArchiveMember, idx: usize) -> Entry {
    let meta = &m.meta;
    let crc32 = match &m.detail {
        _ if meta.is_directory => None,
        ArchiveMemberDetail::Rar15To40 { crc32, .. } => Some(*crc32),
        ArchiveMemberDetail::Rar50Plus { crc32, .. } => *crc32,
        _ => None,
    };
    Entry {
        path: decode_name(&meta.name, m.name_is_unicode(), meta.family),
        is_dir: meta.is_directory,
        size: if meta.is_directory { 0 } else { meta.unpacked_size },
        packed_size: if meta.is_directory { None } else { Some(meta.packed_size) },
        modified: meta.modification_time().and_then(system_time_to_unix),
        crc32,
        encrypted: meta.is_encrypted,
        attributes: Some((meta.file_attr & 0xFFFF) as u32),
        method: Some(if meta.is_stored { "Store" } else { "RAR" }.to_string()),
        index: idx,
    }
}

fn entries_of(members: &[ArchiveMember]) -> Vec<Entry> {
    members
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.meta.is_redirection)
        .map(|(i, m)| member_to_entry(m, i))
        .filter(|e| !e.path.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Errors

fn map_err(e: rars::Error, pw: Option<&str>) -> Error {
    use rars::ErrorKind as K;
    match e.kind() {
        K::PasswordRequired => {
            if pw_bytes(pw).is_some() {
                Error::WrongPassword
            } else {
                Error::PasswordRequired
            }
        }
        K::BadPassword => {
            if pw_bytes(pw).is_none() {
                Error::PasswordRequired
            } else {
                Error::WrongPassword
            }
        }
        K::Cancelled => Error::Cancelled,
        K::ChecksumMismatch => Error::Corrupt(e.to_string()),
        K::InvalidArchive => Error::Corrupt(e.to_string()),
        K::UnsupportedFormat => Error::UnknownFormat,
        K::UnsupportedFeature => Error::Unsupported(e.to_string()),
        _ => Error::Other(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Writers

/// File writer that reports progress per write, aborts on cancellation and
/// remembers a deferred flush error (rars drops writers without flushing).
struct ProgressFile {
    inner: Option<BufWriter<File>>,
    progress: Progress,
    written: Arc<Mutex<u64>>,
    flush_err: Arc<Mutex<Option<io::Error>>>,
}

impl Write for ProgressFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(cancelled_io());
        }
        let n = self.inner.as_mut().map_or(Ok(buf.len()), |w| w.write(buf))?;
        self.progress.add_bytes(n as u64);
        if let Ok(mut w) = self.written.lock() {
            *w += n as u64;
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.as_mut().map_or(Ok(()), |w| w.flush())
    }
}

impl Drop for ProgressFile {
    fn drop(&mut self) {
        if let Some(mut w) = self.inner.take() {
            if let Err(e) = w.flush() {
                if let Ok(mut slot) = self.flush_err.lock() {
                    slot.get_or_insert(e);
                }
            }
        }
    }
}

/// Shared state of one extract/test run.
struct Run<'a> {
    planner: Option<&'a OutputPlanner<'a>>,
    opts_mtime: bool,
    progress: &'a Progress,
    family: ArchiveFamily,
    solid: bool,
    stats: OpStats,
    report: TestReport,
    /// File being written right now and its mtime, finalised on the next entry.
    current: Option<(Option<PathBuf>, Option<i64>)>,
    written: Arc<Mutex<u64>>,
    flush_err: Arc<Mutex<Option<io::Error>>>,
    own_err: Option<Error>,
    mtimes: Vec<(PathBuf, Option<i64>)>,
    /// Members rars reported as broken (test mode keeps going past them).
    failed: Arc<Mutex<Vec<(String, String)>>>,
    failed_seen: usize,
}

impl<'a> Run<'a> {
    fn new(planner: Option<&'a OutputPlanner<'a>>, preserve_mtime: bool, p: &'a Progress, set: &RarSet) -> Self {
        Run {
            planner,
            opts_mtime: preserve_mtime,
            progress: p,
            family: set.family(),
            solid: set.vols.iter().any(is_solid),
            stats: OpStats::default(),
            report: TestReport::default(),
            current: None,
            written: Arc::new(Mutex::new(0)),
            flush_err: Arc::new(Mutex::new(None)),
            own_err: None,
            mtimes: Vec::new(),
            failed: Arc::new(Mutex::new(Vec::new())),
            failed_seen: 0,
        }
    }

    /// Book-keeping for the entry whose writer rars just finished with.
    fn finish_current(&mut self) {
        if let Some((target, mtime)) = self.current.take() {
            let failed = self.failed.lock().map(|g| g.len()).unwrap_or(0);
            if failed > self.failed_seen {
                // rars gave up on this member and moved on.
                self.failed_seen = failed;
                if let Some(t) = &target {
                    let _ = fs::remove_file(t);
                }
                self.progress.file_done();
                return;
            }
            let n = self.written.lock().map(|g| *g).unwrap_or(0);
            match target {
                Some(t) => {
                    self.stats.files += 1;
                    self.stats.bytes += n;
                    self.stats.outputs.push(t.clone());
                    if self.opts_mtime {
                        self.mtimes.push((t, mtime));
                    }
                }
                None => {
                    self.report.files_tested += 1;
                    self.report.bytes_tested += n;
                }
            }
            self.progress.file_done();
        }
    }

    /// Remove the half-written output after a failure.
    fn abandon_current(&mut self) {
        if let Some((Some(t), _)) = self.current.take() {
            let _ = fs::remove_file(&t);
            self.stats.outputs.retain(|o| o != &t);
        }
    }

    /// Decide what to do with one entry. `None` means "skip" (no decoding
    /// needed); `Some(w)` must be handed to rars.
    fn open(&mut self, name: String, is_dir: bool, size: Option<u64>, mtime: Option<i64>) -> Result<Option<Box<dyn Write>>> {
        self.finish_current();
        self.progress.check()?;
        let wanted = !name.is_empty() && self.planner.is_none_or(|pl| pl.wants(&name));
        if !wanted {
            return Ok(None);
        }
        if is_dir {
            if let Some(pl) = self.planner {
                self.progress.begin_file(&name);
                crate::formats::make_dir(pl, &name, mtime, &mut self.stats)?;
                self.progress.file_done();
            }
            return Ok(None);
        }
        self.progress.begin_file(&name);
        let target = match self.planner {
            None => None,
            Some(pl) => match pl.file_path(&name, size, mtime)? {
                Planned::Skip => {
                    self.stats.skipped += 1;
                    self.progress.file_done();
                    return Ok(None);
                }
                Planned::Write(t) => Some(t),
            },
        };
        let file = match &target {
            Some(t) => Some(BufWriter::with_capacity(256 * 1024, File::create(t)?)),
            None => None,
        };
        self.written = Arc::new(Mutex::new(0));
        self.current = Some((target, mtime));
        Ok(Some(Box::new(ProgressFile {
            inner: file,
            progress: self.progress.clone(),
            written: self.written.clone(),
            flush_err: self.flush_err.clone(),
        })))
    }

    /// Adapter for rars callbacks: stash our own error, hand rars a marker.
    fn stash(&mut self, r: Result<Option<Box<dyn Write>>>) -> rars::Result<Option<Box<dyn Write>>> {
        r.map_err(|e| {
            self.own_err = Some(e);
            rars::Error::Cancelled
        })
    }

    fn take_flush_err(&self) -> Option<io::Error> {
        self.flush_err.lock().ok().and_then(|mut g| g.take())
    }

    fn close(&mut self) -> Result<()> {
        self.finish_current();
        if let Some(e) = self.take_flush_err() {
            return Err(e.into());
        }
        for (t, m) in self.mtimes.drain(..) {
            set_mtime(&t, m);
        }
        Ok(())
    }
}

/// Walk every entry of `set`, routing data through `run`. Single archives
/// use rars' controlled extraction (skips unselected non-solid members
/// without decoding, continues past broken ones when `keep_going`);
/// multi-volume sets go through the volume API.
fn walk(set: &RarSet, pw: Option<&str>, run: &mut Run<'_>, keep_going: bool) -> Result<()> {
    // RAR5 members up to this size are decoded in memory and then written in
    // one go; larger ones stream (bounded memory, smooth byte progress).
    let options = ArchiveReadOptions::with_optional_password(pw_bytes(pw))
        .with_rar50_buffered_decode_limit(RAR5_BUFFERED_LIMIT);
    let family = run.family;
    let res = if set.vols.len() == 1 {
        let failed = run.failed.clone();
        let mut decide = |m: &ArchiveMember| -> rars::Result<ExtractionDecision> {
            if m.meta.is_redirection {
                return Ok(ExtractionDecision::Skip);
            }
            let name = decode_name(&m.meta.name, m.name_is_unicode(), family);
            let mtime = m.meta.modification_time().and_then(system_time_to_unix);
            let solid = run.solid;
            let r = run.open(name, m.meta.is_directory, Some(m.meta.unpacked_size), mtime);
            Ok(match run.stash(r)? {
                Some(w) => ExtractionDecision::Extract(w),
                None if solid && !m.meta.is_directory => ExtractionDecision::Extract(Box::new(io::sink())),
                None => ExtractionDecision::Skip,
            })
        };
        if keep_going {
            set.vols[0]
                .extract_with_control_and_errors(options, &mut decide, |m, e| {
                    if e.kind() == rars::ErrorKind::Cancelled
                        || matches!(e.kind(), rars::ErrorKind::PasswordRequired | rars::ErrorKind::BadPassword)
                    {
                        return Ok(ExtractionErrorAction::Abort);
                    }
                    let name = decode_name(&m.meta.name, m.name_is_unicode(), family);
                    if let Ok(mut f) = failed.lock() {
                        f.push((name, e.to_string()));
                    }
                    Ok(ExtractionErrorAction::Continue)
                })
                .map(|_| ())
        } else {
            set.vols[0].extract_with_control(options, &mut decide).map(|_| ())
        }
    } else {
        rars::extract_volumes_to_with_options(&set.vols, options, |meta: &ExtractedEntryMeta| {
            let name = decode_name(&meta.name, meta.name_is_unicode, family);
            let mtime = raw_time_to_unix(meta.file_time, family);
            let r = run.open(name, meta.is_directory, None, mtime);
            Ok(run.stash(r)?.unwrap_or_else(|| Box::new(io::sink())))
        })
    };
    let res = match res {
        Ok(()) => run.close(),
        Err(e) => {
            run.abandon_current();
            if let Some(own) = run.own_err.take() {
                return Err(own);
            }
            if run.progress.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if let Some(io) = run.take_flush_err() {
                return Err(io.into());
            }
            Err(map_err(e, pw))
        }
    };
    let failed = run.failed.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    run.report.errors.extend(failed);
    res
}

// ---------------------------------------------------------------------------
// Public backend API

pub(crate) fn list(path: &Path, opts: &OpenOptions, _p: &Progress) -> Result<ArchiveInfo> {
    let set = RarSet::open(path, opts.password.as_deref())?;
    let entries = entries_of(&set.members()?);
    let encrypted_headers = set.vols.iter().any(has_encrypted_headers);
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        format: ArchiveFormat::Rar,
        encrypted: encrypted_headers || entries.iter().any(|e| e.encrypted),
        solid: set.vols.iter().any(is_solid),
        volumes: set.paths.len(),
        archive_size: set.total_size(),
        entries,
    })
}

/// A CRC failure on an encrypted member is almost always a wrong password
/// (RAR 3/4 has no password check value).
fn refine(e: Error, encrypted: bool, pw: Option<&str>) -> Error {
    match e {
        Error::Corrupt(_) if encrypted && pw_bytes(pw).is_some() => Error::WrongPassword,
        other => other,
    }
}

pub(crate) fn extract(path: &Path, dest: &Path, opts: &ExtractOptions, p: &Progress) -> Result<OpStats> {
    let pw = opts.password.as_deref();
    let set = RarSet::open(path, pw)?;
    let entries = entries_of(&set.members()?);
    let planner = OutputPlanner::new(dest, opts);
    let chosen: Vec<&Entry> = entries.iter().filter(|e| planner.wants(&e.path)).collect();
    let need_pw = chosen.iter().any(|e| e.encrypted);
    if need_pw && pw_bytes(pw).is_none() {
        return Err(Error::PasswordRequired);
    }
    p.set_totals(chosen.iter().map(|e| e.size).sum(), chosen.len() as u64);
    let mut run = Run::new(Some(&planner), opts.preserve_mtime, p, &set);
    walk(&set, pw, &mut run, false).map_err(|e| refine(e, need_pw, pw))?;
    p.finish();
    Ok(run.stats)
}

pub(crate) fn test(path: &Path, pw: Option<&str>, p: &Progress) -> Result<TestReport> {
    let set = RarSet::open(path, pw)?;
    let entries = entries_of(&set.members()?);
    let encrypted = entries.iter().any(|e| e.encrypted);
    if encrypted && pw_bytes(pw).is_none() {
        return Err(Error::PasswordRequired);
    }
    let files: Vec<&Entry> = entries.iter().filter(|e| !e.is_dir).collect();
    p.set_totals(files.iter().map(|e| e.size).sum(), files.len() as u64);
    let mut run = Run::new(None, false, p, &set);
    match walk(&set, pw, &mut run, true).map_err(|e| refine(e, encrypted, pw)) {
        Ok(()) => {}
        Err(e @ (Error::Cancelled | Error::PasswordRequired | Error::WrongPassword)) => return Err(e),
        Err(e) => run.report.errors.push(("(archive)".into(), e.to_string())),
    }
    // Per-member CRC failures on encrypted data mean a wrong password too.
    if encrypted && !run.report.errors.is_empty() && run.report.files_tested == 0 {
        return Err(Error::WrongPassword);
    }
    p.finish();
    Ok(run.report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names() {
        assert_eq!(part_naming("a.part1.rar"), Some(("a".into(), 1)));
        assert_eq!(part_naming("My.Set.PART007.RAR"), Some(("My.Set".into(), 3)));
        assert_eq!(part_naming("a.rar"), None);
        assert_eq!(part_naming("a.partx.rar"), None);
    }
}
