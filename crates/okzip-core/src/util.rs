//! Shared helpers: path safety, conflicts, timestamps, source collection.

use crate::error::{Error, Result};
use crate::types::{ConflictChoice, ConflictInfo, ExtractOptions, OverwriteMode};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

/// Normalize an archive path: `\` → `/`, drop `./`, leading `/`, trailing `/`.
pub fn normalize_entry_path(p: &str) -> String {
    let p = p.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Turn an archive path into a safe relative filesystem path: removes drive
/// prefixes, `..`, and characters Windows forbids. Returns `None` if nothing
/// usable is left.
pub fn sanitize_rel_path(entry: &str) -> Option<PathBuf> {
    let norm = normalize_entry_path(entry);
    let mut out = PathBuf::new();
    for comp in norm.split('/') {
        if comp == ".." || comp == "." || comp.is_empty() {
            continue;
        }
        // Strip a drive letter like "C:".
        let comp = if comp.len() == 2 && comp.ends_with(':') { continue } else { comp };
        let cleaned: String = comp
            .chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
                c if (c as u32) < 0x20 => '_',
                c => c,
            })
            .collect();
        let cleaned = cleaned.trim_end_matches([' ', '.']).to_string();
        if cleaned.is_empty() {
            continue;
        }
        out.push(cleaned);
    }
    // Final check against anything absolute slipping through.
    if out.as_os_str().is_empty()
        || out.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(out)
}

/// Does `path` fall under one of the selected entry paths?
pub(crate) fn is_selected(path: &str, selection: &Option<Vec<String>>) -> bool {
    match selection {
        None => true,
        Some(sel) => sel.iter().any(|s| {
            let s = s.trim_end_matches('/');
            s.is_empty() || path == s || (path.len() > s.len() && path.starts_with(s) && path.as_bytes()[s.len()] == b'/')
        }),
    }
}

/// Decides where (and whether) each entry gets written during extraction.
pub(crate) struct OutputPlanner<'a> {
    dest: PathBuf,
    opts: &'a ExtractOptions,
    sticky: AtomicU8, // 0 = none, 1 = overwrite all, 2 = skip all, 3 = rename all
}

pub(crate) enum Planned {
    Write(PathBuf),
    Skip,
}

impl<'a> OutputPlanner<'a> {
    pub(crate) fn new(dest: &Path, opts: &'a ExtractOptions) -> Self {
        OutputPlanner { dest: dest.to_path_buf(), opts, sticky: AtomicU8::new(0) }
    }

    pub(crate) fn wants(&self, entry_path: &str) -> bool {
        is_selected(entry_path, &self.opts.selection)
    }

    fn relative(&self, entry_path: &str) -> Option<PathBuf> {
        let mut p = normalize_entry_path(entry_path);
        if let Some(prefix) = &self.opts.strip_prefix {
            let prefix = normalize_entry_path(prefix);
            if !prefix.is_empty() {
                if p == prefix {
                    return None;
                }
                if let Some(rest) = p.strip_prefix(&format!("{prefix}/")) {
                    p = rest.to_string();
                }
            }
        }
        sanitize_rel_path(&p)
    }

    /// Directory target (always created, never conflicts).
    pub(crate) fn dir_path(&self, entry_path: &str) -> Option<PathBuf> {
        self.relative(entry_path).map(|r| self.dest.join(r))
    }

    /// File target after applying the overwrite policy. Creates parent dirs.
    /// `new_size` / `new_mtime` describe the incoming entry (shown when asking).
    pub(crate) fn file_path(&self, entry_path: &str, new_size: Option<u64>, new_mtime: Option<i64>) -> Result<Planned> {
        let Some(rel) = self.relative(entry_path) else { return Ok(Planned::Skip) };
        let target = self.dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if !target.exists() {
            return Ok(Planned::Write(target));
        }
        if target.is_dir() {
            // A directory is in the way; never clobber it.
            return Ok(Planned::Write(unique_path(&target)));
        }
        let mode = match self.sticky.load(Ordering::Relaxed) {
            1 => OverwriteMode::Overwrite,
            2 => OverwriteMode::Skip,
            3 => OverwriteMode::Rename,
            _ => self.opts.overwrite,
        };
        match mode {
            OverwriteMode::Overwrite => Ok(Planned::Write(target)),
            OverwriteMode::Skip => Ok(Planned::Skip),
            OverwriteMode::Rename => Ok(Planned::Write(unique_path(&target))),
            OverwriteMode::Ask => {
                let choice = match &self.opts.on_conflict {
                    Some(cb) => cb(&ConflictInfo { target: &target, entry_path, new_size, new_modified: new_mtime }),
                    None => ConflictChoice::Skip,
                };
                match choice {
                    ConflictChoice::Overwrite => Ok(Planned::Write(target)),
                    ConflictChoice::OverwriteAll => {
                        self.sticky.store(1, Ordering::Relaxed);
                        Ok(Planned::Write(target))
                    }
                    ConflictChoice::Skip => Ok(Planned::Skip),
                    ConflictChoice::SkipAll => {
                        self.sticky.store(2, Ordering::Relaxed);
                        Ok(Planned::Skip)
                    }
                    ConflictChoice::Rename => Ok(Planned::Write(unique_path(&target))),
                    ConflictChoice::RenameAll => {
                        self.sticky.store(3, Ordering::Relaxed);
                        Ok(Planned::Write(unique_path(&target)))
                    }
                    ConflictChoice::Cancel => Err(Error::Cancelled),
                }
            }
        }
    }
}

/// "name.ext" → "name (2).ext", "name (3).ext", ... first that doesn't exist.
pub fn unique_path(p: &Path) -> PathBuf {
    let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
    let file = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (file[..i].to_string(), file[i..].to_string()),
        _ => (file.clone(), String::new()),
    };
    for n in 2..100_000 {
        let cand = parent.join(format!("{stem} ({n}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    p.to_path_buf()
}

/// Set a file's mtime from Unix seconds (best effort).
pub(crate) fn set_mtime(path: &Path, unix: Option<i64>) {
    if let Some(t) = unix {
        let ft = filetime::FileTime::from_unix_time(t, 0);
        let _ = filetime::set_file_mtime(path, ft);
    }
}

pub(crate) fn system_time_to_unix(t: std::time::SystemTime) -> Option<i64> {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => Some(d.as_secs() as i64),
        Err(e) => Some(-(e.duration().as_secs() as i64)),
    }
}

/// Windows FILETIME (100 ns since 1601) → Unix seconds.
pub(crate) fn filetime_to_unix(ft: u64) -> Option<i64> {
    if ft == 0 {
        return None;
    }
    Some((ft / 10_000_000) as i64 - 11_644_473_600)
}

fn local_offset() -> time::UtcOffset {
    time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC)
}

/// DOS-style civil time (interpreted as local time) → Unix seconds.
pub(crate) fn civil_local_to_unix(y: i32, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> Option<i64> {
    let month = time::Month::try_from(mo).ok()?;
    let date = time::Date::from_calendar_date(y, month, d).ok()?;
    let t = time::Time::from_hms(h, mi, s.min(59)).ok()?;
    let pdt = time::PrimitiveDateTime::new(date, t);
    Some(pdt.assume_offset(local_offset()).unix_timestamp())
}

/// Unix seconds → local civil time.
pub(crate) fn unix_to_local_civil(unix: i64) -> Option<time::PrimitiveDateTime> {
    let odt = time::OffsetDateTime::from_unix_timestamp(unix).ok()?.to_offset(local_offset());
    Some(time::PrimitiveDateTime::new(odt.date(), odt.time()))
}

/// DOS/MS-DOS date+time words (as stored by RAR) → Unix seconds.
#[cfg_attr(not(feature = "rar"), allow(dead_code))]
pub(crate) fn dos_to_unix(dos: u32) -> Option<i64> {
    let date = (dos >> 16) as u16;
    let tm = (dos & 0xFFFF) as u16;
    let y = 1980 + (date >> 9) as i32;
    let mo = ((date >> 5) & 0xF) as u8;
    let d = (date & 0x1F) as u8;
    let h = (tm >> 11) as u8;
    let mi = ((tm >> 5) & 0x3F) as u8;
    let s = ((tm & 0x1F) * 2) as u8;
    civil_local_to_unix(y, mo, d, h, mi, s)
}

/// A file or directory on disk scheduled to go into a new archive.
#[derive(Clone, Debug)]
pub struct SourceItem {
    pub fs_path: PathBuf,
    /// Archive-relative path with `/` separators.
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
}

/// Expand the given paths into archive items. Each source keeps its own name
/// at the archive root (or under `prefix`). Paths equal to `exclude` (the
/// archive being written) are skipped.
pub fn collect_sources(sources: &[PathBuf], prefix: &str, exclude: &[PathBuf]) -> Result<Vec<SourceItem>> {
    let prefix = normalize_entry_path(prefix);
    let excluded: Vec<PathBuf> = exclude.iter().filter_map(|p| fs::canonicalize(p).ok()).collect();
    let mut items = Vec::new();
    for src in sources {
        let meta = fs::metadata(src)?;
        let base = src
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .ok_or_else(|| Error::Other(format!("invalid source path: {}", src.display())))?;
        let root = if prefix.is_empty() { base.clone() } else { format!("{prefix}/{base}") };
        if meta.is_dir() {
            for ent in walkdir::WalkDir::new(src).sort_by_file_name().follow_links(false) {
                let ent = ent.map_err(|e| Error::Other(e.to_string()))?;
                let path = ent.path();
                if let Ok(c) = fs::canonicalize(path) {
                    if excluded.contains(&c) {
                        continue;
                    }
                }
                let rel = path.strip_prefix(src).unwrap_or(path);
                let rel_s = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                let name = if rel_s.is_empty() { root.clone() } else { format!("{root}/{rel_s}") };
                let m = ent.metadata().map_err(|e| Error::Other(e.to_string()))?;
                if m.file_type().is_symlink() {
                    continue; // v0.1: symlinks are not archived
                }
                items.push(SourceItem {
                    fs_path: path.to_path_buf(),
                    name,
                    is_dir: m.is_dir(),
                    size: if m.is_dir() { 0 } else { m.len() },
                    modified: m.modified().ok().and_then(system_time_to_unix),
                });
            }
        } else {
            if let Ok(c) = fs::canonicalize(src) {
                if excluded.contains(&c) {
                    continue;
                }
            }
            items.push(SourceItem {
                fs_path: src.clone(),
                name: root,
                is_dir: false,
                size: meta.len(),
                modified: meta.modified().ok().and_then(system_time_to_unix),
            });
        }
    }
    Ok(items)
}

/// Decode a raw ZIP/legacy file name: UTF-8 when valid, otherwise GBK
/// (the common case for archives made on Chinese Windows).
pub(crate) fn decode_legacy_name(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(raw);
            cow.into_owned()
        }
    }
}

/// A temporary directory next to `near` that is removed on drop.
pub(crate) struct TempDir(pub PathBuf);

impl TempDir {
    pub(crate) fn new_near(near: &Path) -> Result<TempDir> {
        let parent = near.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = parent.join(format!(".okzip-tmp-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p)?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Recursively copy a file or directory into `dest_dir`.
pub(crate) fn copy_into(src: &Path, dest_dir: &Path) -> Result<()> {
    let name = src.file_name().ok_or_else(|| Error::Other("bad source".into()))?;
    let target = dest_dir.join(name);
    if src.is_dir() {
        fs::create_dir_all(&target)?;
        for ent in fs::read_dir(src)? {
            copy_into(&ent?.path(), &target)?;
        }
    } else {
        if target.is_dir() {
            fs::remove_dir_all(&target)?;
        }
        fs::copy(src, &target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_blocks_traversal() {
        assert_eq!(sanitize_rel_path("../../etc/passwd"), Some(PathBuf::from("etc/passwd")));
        assert_eq!(sanitize_rel_path("/abs/x"), Some(PathBuf::from("abs/x")));
        assert_eq!(sanitize_rel_path("C:\\win\\x.dll"), Some(PathBuf::from("win/x.dll")));
        assert_eq!(sanitize_rel_path(".."), None);
        assert_eq!(sanitize_rel_path("a/b?.txt"), Some(PathBuf::from("a/b_.txt")));
    }

    #[test]
    fn selection_matches_subtree() {
        let sel = Some(vec!["dir".to_string()]);
        assert!(is_selected("dir", &sel));
        assert!(is_selected("dir/a.txt", &sel));
        assert!(!is_selected("dir2/a.txt", &sel));
        assert!(is_selected("anything", &None));
    }

    #[test]
    fn gbk_names_decode() {
        // "中文.txt" in GBK
        let raw = [0xD6, 0xD0, 0xCE, 0xC4, b'.', b't', b'x', b't'];
        assert_eq!(decode_legacy_name(&raw), "中文.txt");
    }
}
