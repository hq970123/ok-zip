//! RAR backend (list/extract/test only) via the `unrar` crate (UnRAR C++ source).
//! Progress is per entry; cancellation is checked between entries.

use crate::error::{Error, Result};
use crate::progress::Progress;
use crate::types::{ArchiveFormat, ArchiveInfo, Entry, ExtractOptions, OpStats, OpenOptions, TestReport};
use crate::util::{dos_to_unix, normalize_entry_path, set_mtime, OutputPlanner, Planned};
use std::path::Path;
use unrar::Archive;

fn archive<'a>(path: &'a Path, pw: &'a Option<String>) -> Archive<'a> {
    match pw.as_deref().filter(|s| !s.is_empty()) {
        Some(p) => Archive::with_password(path, p.as_bytes()),
        None => Archive::new(path),
    }
}

fn header_to_entry(h: &unrar::FileHeader, idx: usize) -> Entry {
    let path = normalize_entry_path(&h.filename.to_string_lossy());
    Entry {
        path,
        is_dir: h.is_directory(),
        size: if h.is_directory() { 0 } else { h.unpacked_size },
        packed_size: None,
        modified: dos_to_unix(h.file_time),
        crc32: if h.is_directory() { None } else { Some(h.file_crc) },
        encrypted: h.is_encrypted(),
        attributes: Some(h.file_attr & 0xFFFF),
        method: None,
        index: idx,
    }
}

fn volume_files(path: &Path) -> (u64, usize) {
    let a = Archive::new(path);
    if a.is_multipart() {
        let mut total = 0;
        let mut n = 0;
        for i in 1..1000 {
            match a.nth_part(i) {
                Some(p) if p.is_file() => {
                    total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    n += 1;
                }
                _ => break,
            }
        }
        if n > 0 {
            return (total, n);
        }
    }
    (std::fs::metadata(path).map(|m| m.len()).unwrap_or(0), 1)
}

pub(crate) fn list(path: &Path, opts: &OpenOptions, _p: &Progress) -> Result<ArchiveInfo> {
    let pw = opts.password.clone();
    let mut open = archive(path, &pw).open_for_listing()?;
    let encrypted_headers = open.has_encrypted_headers();
    let solid = open.is_solid();
    let mut entries = Vec::new();
    let mut idx = 0;
    while let Some(header) = open.read_header()? {
        entries.push(header_to_entry(header.entry(), idx));
        idx += 1;
        open = header.skip()?;
    }
    let (size, vols) = volume_files(path);
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        format: ArchiveFormat::Rar,
        encrypted: encrypted_headers || entries.iter().any(|e| e.encrypted),
        entries,
        solid,
        volumes: vols,
        archive_size: size,
    })
}

pub(crate) fn extract(path: &Path, dest: &Path, opts: &ExtractOptions, p: &Progress) -> Result<OpStats> {
    let info = list(path, &OpenOptions { password: opts.password.clone() }, p)?;
    let planner = OutputPlanner::new(dest, opts);
    let chosen: Vec<&Entry> = info.entries.iter().filter(|e| planner.wants(&e.path)).collect();
    if chosen.iter().any(|e| e.encrypted) && opts.password.as_deref().is_none_or(str::is_empty) {
        return Err(Error::PasswordRequired);
    }
    p.set_totals(chosen.iter().map(|e| e.size).sum(), chosen.len() as u64);
    let mut stats = OpStats::default();
    let pw = opts.password.clone();
    let mut open = archive(path, &pw).open_for_processing()?;
    let mut done_bytes = 0u64;
    while let Some(header) = open.read_header()? {
        p.check()?;
        let e = header_to_entry(header.entry(), 0);
        if e.path.is_empty() || !planner.wants(&e.path) {
            open = header.skip()?;
            continue;
        }
        p.begin_file(&e.path);
        if e.is_dir {
            crate::formats::make_dir(&planner, &e.path, e.modified, &mut stats)?;
            open = header.skip()?;
        } else {
            match planner.file_path(&e.path)? {
                Planned::Skip => {
                    stats.skipped += 1;
                    open = header.skip()?;
                }
                Planned::Write(target) => {
                    open = header.extract_to(&target)?;
                    if opts.preserve_mtime {
                        set_mtime(&target, e.modified);
                    }
                    stats.files += 1;
                    stats.bytes += e.size;
                    stats.outputs.push(target);
                }
            }
            done_bytes += e.size;
            p.set_bytes(done_bytes);
        }
        p.file_done();
    }
    p.finish();
    Ok(stats)
}

pub(crate) fn test(path: &Path, pw: Option<&str>, p: &Progress) -> Result<TestReport> {
    let pw = pw.map(str::to_string);
    let info = list(path, &OpenOptions { password: pw.clone() }, p)?;
    if info.encrypted && pw.as_deref().is_none_or(str::is_empty) {
        return Err(Error::PasswordRequired);
    }
    p.set_totals(info.total_size(), info.file_count() as u64);
    let mut report = TestReport::default();
    let mut open = archive(path, &pw).open_for_processing()?;
    let mut done = 0;
    loop {
        p.check()?;
        let header = match open.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => {
                report.errors.push(("(archive)".into(), e.to_string()));
                break;
            }
        };
        let e = header_to_entry(header.entry(), 0);
        p.begin_file(&e.path);
        if e.is_dir {
            open = header.skip()?;
            continue;
        }
        match header.test() {
            Ok(next) => {
                open = next;
                report.files_tested += 1;
                report.bytes_tested += e.size;
            }
            Err(err) => {
                let mapped: Error = err.into();
                if mapped.needs_password() {
                    return Err(mapped);
                }
                report.errors.push((e.path.clone(), mapped.to_string()));
                break; // the handle is consumed; RAR can't continue past a fatal error
            }
        }
        done += e.size;
        p.set_bytes(done);
        p.file_done();
    }
    p.finish();
    Ok(report)
}
