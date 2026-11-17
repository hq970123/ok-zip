//! TAR family (plain, gz, bz2, xz, zst) and bare single-file streams.

use crate::detect::archive_stem;
use crate::error::{Error, Result};
use crate::formats::{make_dir, write_entry};
use crate::progress::{PositionReader, Progress, ProgressReader};
use crate::types::{ArchiveFormat, ArchiveInfo, CreateOptions, Entry, ExtractOptions, OpStats, OpenOptions, TestReport};
use crate::util::{normalize_entry_path, OutputPlanner, SourceItem};
use crate::volume::open_reader;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Wrap `r` in the decompressor for `fmt` (identity for plain tar).
pub(crate) fn decoder<'a, R: Read + Send + 'a>(fmt: ArchiveFormat, r: R) -> Result<Box<dyn Read + Send + 'a>> {
    Ok(match fmt {
        ArchiveFormat::Tar => Box::new(r),
        ArchiveFormat::Gz | ArchiveFormat::TarGz => Box::new(flate2::read::MultiGzDecoder::new(BufReader::new(r))),
        ArchiveFormat::Bz2 | ArchiveFormat::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(BufReader::new(r))),
        ArchiveFormat::Xz | ArchiveFormat::TarXz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(BufReader::new(r))),
        ArchiveFormat::Zst | ArchiveFormat::TarZst => Box::new(zstd::stream::read::Decoder::new(r)?),
        other => return Err(Error::Unsupported(format!("{other} is not a stream format"))),
    })
}

fn inner_format(fmt: ArchiveFormat) -> ArchiveFormat {
    match fmt {
        ArchiveFormat::TarGz => ArchiveFormat::Gz,
        ArchiveFormat::TarBz2 => ArchiveFormat::Bz2,
        ArchiveFormat::TarXz => ArchiveFormat::Xz,
        ArchiveFormat::TarZst => ArchiveFormat::Zst,
        f => f,
    }
}

/// Open the raw file and wrap it: position progress on the compressed bytes.
fn open_stream<'a>(path: &Path, fmt: ArchiveFormat, p: &Progress) -> Result<(Box<dyn Read + Send + 'a>, u64)> {
    let (r, size, _) = open_reader(path)?;
    let raw = PositionReader::new(r, p);
    let fmt = if fmt == ArchiveFormat::Tar { ArchiveFormat::Tar } else { inner_format(fmt) };
    Ok((decoder(fmt, raw)?, size))
}

fn tar_entries<R: Read>(ar: &mut ::tar::Archive<R>) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for (i, ent) in ar.entries()?.enumerate() {
        let ent = ent?;
        let h = ent.header();
        let path = normalize_entry_path(&String::from_utf8_lossy(&ent.path_bytes()));
        if path.is_empty() {
            continue;
        }
        let kind = h.entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() || kind.is_gnu_longname() || kind.is_gnu_longlink() {
            continue;
        }
        let is_dir = kind.is_dir();
        let mode = h.mode().ok();
        let mut attr = if is_dir { 0x10 } else { 0x20 };
        if mode.is_some_and(|m| m & 0o200 == 0) {
            attr |= 0x01;
        }
        out.push(Entry {
            path,
            is_dir,
            size: if is_dir { 0 } else { ent.size() },
            packed_size: None,
            modified: h.mtime().ok().map(|t| t as i64),
            crc32: None,
            encrypted: false,
            attributes: Some(attr),
            method: Some(if kind.is_symlink() { "Symlink".into() } else { String::new() }).filter(|s: &String| !s.is_empty()),
            index: i,
        });
    }
    Ok(out)
}

/// gzip: read the original name (FNAME) and ISIZE trailer.
fn gzip_meta(path: &Path) -> (Option<String>, Option<u64>, Option<i64>) {
    let Ok(mut f) = File::open(path) else { return (None, None, None) };
    let mut head = [0u8; 10];
    if f.read_exact(&mut head).is_err() || head[0] != 0x1F || head[1] != 0x8B {
        return (None, None, None);
    }
    let flags = head[3];
    let mtime = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    let mut name = None;
    let mut r = BufReader::new(&mut f);
    if flags & 0x04 != 0 {
        let mut xlen = [0u8; 2];
        if r.read_exact(&mut xlen).is_ok() {
            let mut skip = vec![0u8; u16::from_le_bytes(xlen) as usize];
            let _ = r.read_exact(&mut skip);
        }
    }
    if flags & 0x08 != 0 {
        let mut bytes = Vec::new();
        let mut b = [0u8; 1];
        while r.read_exact(&mut b).is_ok() && b[0] != 0 && bytes.len() < 4096 {
            bytes.push(b[0]);
        }
        // FNAME is ISO-8859-1 by spec but often UTF-8/GBK in practice.
        name = Some(crate::util::decode_legacy_name(&bytes));
    }
    drop(r);
    let size = (|| -> io::Result<u64> {
        f.seek(SeekFrom::End(-4))?;
        let mut t = [0u8; 4];
        f.read_exact(&mut t)?;
        Ok(u32::from_le_bytes(t) as u64)
    })()
    .ok();
    (name, size, if mtime > 0 { Some(mtime as i64) } else { None })
}

fn single_entry(path: &Path, fmt: ArchiveFormat) -> Entry {
    let (gz_name, gz_size, gz_mtime) = if fmt == ArchiveFormat::Gz { gzip_meta(path) } else { (None, None, None) };
    let name = gz_name
        .map(|n| normalize_entry_path(&n))
        .and_then(|n| n.rsplit('/').next().map(str::to_string))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| archive_stem(path));
    let fs_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(crate::util::system_time_to_unix);
    Entry {
        path: name,
        is_dir: false,
        size: gz_size.unwrap_or(0),
        packed_size: std::fs::metadata(path).ok().map(|m| m.len()),
        modified: gz_mtime.or(fs_mtime),
        crc32: None,
        encrypted: false,
        attributes: Some(0x20),
        method: Some(fmt.name().to_string()),
        index: 0,
    }
}

pub(crate) fn list(path: &Path, fmt: ArchiveFormat, _opts: &OpenOptions, p: &Progress) -> Result<ArchiveInfo> {
    let (_, size, vols) = open_reader(path)?;
    let entries = if fmt.is_single_stream() {
        vec![single_entry(path, fmt)]
    } else {
        p.set_totals(size, 0);
        let (r, _) = open_stream(path, fmt, p)?;
        let mut ar = ::tar::Archive::new(r);
        let e = tar_entries(&mut ar)?;
        p.finish();
        e
    };
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        format: fmt,
        entries,
        encrypted: false,
        solid: fmt != ArchiveFormat::Tar,
        volumes: vols,
        archive_size: size,
    })
}

pub(crate) fn extract(path: &Path, fmt: ArchiveFormat, dest: &Path, opts: &ExtractOptions, p: &Progress) -> Result<OpStats> {
    let planner = OutputPlanner::new(dest, opts);
    let mut stats = OpStats::default();
    if fmt.is_single_stream() {
        let e = single_entry(path, fmt);
        if !planner.wants(&e.path) {
            return Ok(stats);
        }
        p.set_totals(e.size, 1);
        p.begin_file(&e.path);
        let (r, _, _) = open_reader(path)?;
        let mut dec = decoder(fmt, r)?;
        write_entry(&planner, &e.path, &mut dec, (e.size > 0).then_some(e.size), e.modified, opts.preserve_mtime, p, &mut stats)?;
        p.file_done();
        p.finish();
        return Ok(stats);
    }
    // Progress over the compressed stream: we can't know uncompressed size up front cheaply.
    let (_, size, _) = open_reader(path)?;
    p.set_totals(size, 0);
    // `p` tracks the raw (compressed) position; file writes report nowhere.
    let (r, _) = open_stream(path, fmt, p)?;
    let quiet = Progress::new().with_cancel_flag(p.cancel_flag());
    let mut ar = ::tar::Archive::new(r);
    for ent in ar.entries()? {
        p.check()?;
        let mut ent = ent?;
        let name = normalize_entry_path(&String::from_utf8_lossy(&ent.path_bytes()));
        let kind = ent.header().entry_type();
        if name.is_empty() || !planner.wants(&name) {
            continue;
        }
        p.begin_file(&name);
        let mtime = ent.header().mtime().ok().map(|t| t as i64);
        if kind.is_dir() {
            make_dir(&planner, &name, mtime, &mut stats)?;
        } else if kind.is_file() || kind == ::tar::EntryType::Continuous || kind == ::tar::EntryType::GNUSparse {
            let size = ent.header().size().ok();
            write_entry(&planner, &name, &mut ent, size, mtime, opts.preserve_mtime, &quiet, &mut stats)?;
        } else if kind.is_symlink() || kind.is_hard_link() {
            #[cfg(unix)]
            if let Some(target_dir) = planner.dir_path(&name).and_then(|t| t.parent().map(|x| x.to_path_buf())) {
                std::fs::create_dir_all(&target_dir)?;
                let _ = ent.unpack_in(dest);
            }
            stats.skipped += u64::from(cfg!(not(unix)));
        }
        p.file_done();
    }
    p.finish();
    Ok(stats)
}

pub(crate) fn test(path: &Path, fmt: ArchiveFormat, p: &Progress) -> Result<TestReport> {
    let mut report = TestReport::default();
    let (_, size, _) = open_reader(path)?;
    p.set_totals(size, 0);
    let res: Result<()> = (|| {
        if fmt.is_single_stream() {
            let (mut r, _) = open_stream(path, fmt, p)?;
            let n = io::copy(&mut r, &mut io::sink())?;
            report.files_tested = 1;
            report.bytes_tested = n;
            return Ok(());
        }
        let (r, _) = open_stream(path, fmt, p)?;
        let mut ar = ::tar::Archive::new(r);
        for ent in ar.entries()? {
            p.check()?;
            let mut ent = ent?;
            let name = String::from_utf8_lossy(&ent.path_bytes()).to_string();
            p.begin_file(&name);
            let n = io::copy(&mut ent, &mut io::sink())?;
            if !ent.header().entry_type().is_dir() {
                report.files_tested += 1;
                report.bytes_tested += n;
            }
        }
        Ok(())
    })();
    match res {
        Ok(()) => {}
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => report.errors.push(("(archive)".into(), e.to_string())),
    }
    p.finish();
    Ok(report)
}

fn build<W: Write>(w: W, items: &[SourceItem], p: &Progress) -> Result<W> {
    let mut b = ::tar::Builder::new(w);
    b.follow_symlinks(false);
    for item in items {
        p.check()?;
        p.begin_file(&item.name);
        let meta = std::fs::metadata(&item.fs_path)?;
        let mut h = ::tar::Header::new_gnu();
        h.set_metadata(&meta);
        if item.is_dir {
            h.set_entry_type(::tar::EntryType::Directory);
            h.set_size(0);
            b.append_data(&mut h, format!("{}/", item.name), io::empty())?;
        } else {
            h.set_entry_type(::tar::EntryType::Regular);
            h.set_size(item.size);
            let r = ProgressReader::new(File::open(&item.fs_path)?, p).take(item.size);
            b.append_data(&mut h, &item.name, r)?;
        }
        p.file_done();
    }
    Ok(b.into_inner()?)
}

fn zstd_level(level: u32) -> i32 {
    [1, 1, 3, 3, 5, 7, 9, 12, 15, 19][level.min(9) as usize]
}

pub(crate) fn create(items: &[SourceItem], target: &Path, opts: &CreateOptions, p: &Progress) -> Result<()> {
    let file = BufWriter::with_capacity(256 * 1024, File::create(target)?);
    let lvl = opts.level.min(9);
    match opts.format {
        ArchiveFormat::Tar => {
            build(file, items, p)?.flush()?;
        }
        ArchiveFormat::TarGz => {
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::new(lvl));
            build(enc, items, p)?.finish()?.flush()?;
        }
        ArchiveFormat::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(file, bzip2::Compression::new(lvl.max(1)));
            build(enc, items, p)?.finish()?.flush()?;
        }
        ArchiveFormat::TarXz => {
            let enc = liblzma::write::XzEncoder::new(file, lvl);
            build(enc, items, p)?.finish()?.flush()?;
        }
        ArchiveFormat::TarZst => {
            let enc = zstd::stream::write::Encoder::new(file, zstd_level(lvl))?;
            build(enc, items, p)?.finish()?.flush()?;
        }
        other => return Err(Error::Unsupported(format!("cannot create {other} with the tar backend"))),
    }
    Ok(())
}
