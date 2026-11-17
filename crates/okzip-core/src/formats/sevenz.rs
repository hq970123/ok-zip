//! 7z backend via `sevenz-rust2` (read: LZMA/LZMA2/PPMd/BZip2/Deflate/BCJ/AES;
//! write: LZMA2 levels or Copy, optional AES-256 incl. header encryption).

use crate::error::{Error, Result};
use crate::formats::{make_dir, write_entry};
use crate::progress::{copy_progress, Progress, ProgressReader};
use crate::types::{ArchiveFormat, ArchiveInfo, CreateOptions, Entry, ExtractOptions, OpStats, OpenOptions, TestReport};
use crate::util::{filetime_to_unix, normalize_entry_path, OutputPlanner, SourceItem};
use crate::volume::{open_reader, ReadSeek};
use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
use sevenz_rust2::{Archive, ArchiveEntry, ArchiveReader, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password, SourceReader};
use std::fs::File;
use std::io;
use std::path::Path;

fn password(pw: Option<&str>) -> Password {
    match pw {
        Some(p) if !p.is_empty() => Password::new(p),
        _ => Password::empty(),
    }
}

fn open(path: &Path, pw: Option<&str>) -> Result<(Archive, Box<dyn ReadSeek>, u64, usize)> {
    let (mut r, size, vols) = open_reader(path)?;
    let archive = Archive::read(&mut r, &password(pw))?;
    Ok((archive, r, size, vols))
}

fn to_entries(a: &Archive) -> Vec<Entry> {
    a.files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_anti_item)
        .map(|(i, f)| Entry {
            path: normalize_entry_path(&f.name),
            is_dir: f.is_directory,
            size: f.size,
            packed_size: None,
            modified: if f.has_last_modified_date { filetime_to_unix(u64::from(f.last_modified_date)) } else { None },
            crc32: if f.has_crc { Some(f.crc as u32) } else { None },
            encrypted: a.is_file_encrypted(i),
            attributes: if f.has_windows_attributes { Some(f.windows_attributes & 0xFFFF) } else { None },
            method: None,
            index: i,
        })
        .filter(|e| !e.path.is_empty())
        .collect()
}

pub(crate) fn list(path: &Path, opts: &OpenOptions, _p: &Progress) -> Result<ArchiveInfo> {
    let (archive, _r, size, vols) = open(path, opts.password.as_deref())?;
    let entries = to_entries(&archive);
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        format: ArchiveFormat::SevenZ,
        encrypted: entries.iter().any(|e| e.encrypted),
        entries,
        solid: archive.is_solid,
        volumes: vols,
        archive_size: size,
    })
}

/// Run `each` over every entry, translating errors in both directions.
fn walk<F>(path: &Path, pw: Option<&str>, mut each: F) -> Result<()>
where
    F: FnMut(&ArchiveEntry, &mut dyn io::Read) -> Result<()>,
{
    let (archive, r, _, _) = open(path, pw)?;
    let encrypted = (0..archive.files.len()).any(|i| archive.is_file_encrypted(i));
    if encrypted && pw.is_none_or(str::is_empty) {
        return Err(Error::PasswordRequired);
    }
    let mut reader = ArchiveReader::from_archive(archive, r, password(pw));
    let mut own_err: Option<Error> = None;
    let res = reader.for_each_entries(|entry, data| match each(entry, data) {
        Ok(()) => Ok(true),
        Err(e) => {
            own_err = Some(e);
            Err(sevenz_rust2::Error::Other("okzip: aborted".into()))
        }
    });
    if let Some(e) = own_err {
        return Err(e);
    }
    res.map_err(|e| {
        let mapped: Error = e.into();
        // A CRC failure on encrypted data is almost always a wrong password.
        match mapped {
            Error::Corrupt(_) if encrypted => Error::WrongPassword,
            other => other,
        }
    })
}

pub(crate) fn extract(path: &Path, dest: &Path, opts: &ExtractOptions, p: &Progress) -> Result<OpStats> {
    let planner = OutputPlanner::new(dest, opts);
    {
        let (archive, _, _, _) = open(path, opts.password.as_deref())?;
        let chosen: Vec<Entry> = to_entries(&archive).into_iter().filter(|e| planner.wants(&e.path)).collect();
        p.set_totals(chosen.iter().map(|e| e.size).sum(), chosen.len() as u64);
    }
    let mut stats = OpStats::default();
    walk(path, opts.password.as_deref(), |entry, data| {
        p.check()?;
        let name = normalize_entry_path(&entry.name);
        if name.is_empty() || entry.is_anti_item || !planner.wants(&name) {
            return Ok(()); // the reader drains unread data itself
        }
        p.begin_file(&name);
        let mtime = if entry.has_last_modified_date { filetime_to_unix(u64::from(entry.last_modified_date)) } else { None };
        if entry.is_directory {
            make_dir(&planner, &name, mtime, &mut stats)?;
        } else {
            write_entry(&planner, &name, data, Some(entry.size), mtime, opts.preserve_mtime, p, &mut stats)?;
        }
        p.file_done();
        Ok(())
    })?;
    p.finish();
    Ok(stats)
}

pub(crate) fn test(path: &Path, pw: Option<&str>, p: &Progress) -> Result<TestReport> {
    {
        let (archive, _, _, _) = open(path, pw)?;
        let files: Vec<Entry> = to_entries(&archive).into_iter().filter(|e| !e.is_dir).collect();
        p.set_totals(files.iter().map(|e| e.size).sum(), files.len() as u64);
    }
    let mut report = TestReport::default();
    let res = walk(path, pw, |entry, data| {
        p.check()?;
        if entry.is_directory {
            return Ok(());
        }
        p.begin_file(&entry.name);
        let n = copy_progress(data, &mut io::sink(), p)?;
        report.files_tested += 1;
        report.bytes_tested += n;
        p.file_done();
        Ok(())
    });
    match res {
        Ok(()) => {}
        Err(e @ (Error::Cancelled | Error::PasswordRequired | Error::WrongPassword)) => return Err(e),
        Err(e) => report.errors.push(("(archive)".into(), e.to_string())),
    }
    p.finish();
    Ok(report)
}

fn methods(opts: &CreateOptions) -> Vec<EncoderConfiguration> {
    let mut v: Vec<EncoderConfiguration> = Vec::new();
    if let Some(pw) = opts.password.as_deref().filter(|s| !s.is_empty()) {
        v.push(AesEncoderOptions::new(Password::new(pw)).into());
    }
    if opts.level == 0 {
        v.push(EncoderMethod::COPY.into());
    } else {
        v.push(Lzma2Options::from_level(opts.level.min(9)).into());
    }
    v
}

fn entry_for(item: &SourceItem) -> ArchiveEntry {
    let mut e = ArchiveEntry::from_path(&item.fs_path, item.name.clone());
    e.is_directory = item.is_dir;
    e.has_stream = !item.is_dir && item.size > 0;
    e
}

pub(crate) fn create(items: &[SourceItem], target: &Path, opts: &CreateOptions, p: &Progress) -> Result<()> {
    let mut w = ArchiveWriter::create(target)?;
    w.set_content_methods(methods(opts));
    if opts.encrypt_headers && opts.password.as_deref().is_some_and(|s| !s.is_empty()) {
        w.set_encrypt_header(true);
    }
    // Directories and empty files carry no stream.
    for item in items.iter().filter(|i| i.is_dir || i.size == 0) {
        p.check()?;
        w.push_archive_entry::<File>(entry_for(item), None)?;
        p.file_done();
    }
    let files: Vec<&SourceItem> = items.iter().filter(|i| !i.is_dir && i.size > 0).collect();
    if opts.solid {
        // One solid block per batch keeps the number of open handles bounded.
        const MAX_FILES: usize = 512;
        const MAX_BYTES: u64 = 512 * 1024 * 1024;
        let mut i = 0;
        while i < files.len() {
            let mut j = i;
            let mut bytes = 0;
            while j < files.len() && j - i < MAX_FILES && (j == i || bytes + files[j].size <= MAX_BYTES) {
                bytes += files[j].size;
                j += 1;
            }
            p.check()?;
            let batch = &files[i..j];
            p.begin_file(&batch[0].name);
            let mut entries = Vec::with_capacity(batch.len());
            let mut readers = Vec::with_capacity(batch.len());
            for item in batch {
                entries.push(entry_for(item));
                readers.push(SourceReader::new(ProgressReader::new(File::open(&item.fs_path)?, p)));
            }
            w.push_archive_entries(entries, readers)?;
            for _ in batch {
                p.file_done();
            }
            i = j;
        }
    } else {
        for item in files {
            p.check()?;
            p.begin_file(&item.name);
            let r = ProgressReader::new(File::open(&item.fs_path)?, p);
            w.push_archive_entry(entry_for(item), Some(r))?;
            p.file_done();
        }
    }
    w.finish()?;
    Ok(())
}
