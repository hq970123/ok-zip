//! ZIP backend (read: Stored/Deflate/Deflate64/BZip2/LZMA/XZ/Zstd/PPMd,
//! ZipCrypto + WinZip AES; write: Deflate levels, AES-256 or ZipCrypto).

use crate::error::{Error, Result};
use crate::formats::{make_dir, write_entry};
use crate::progress::{copy_progress, Progress};
use crate::types::{ArchiveFormat, ArchiveInfo, CreateOptions, Entry, ExtractOptions, OpStats, OpenOptions, TestReport, ZipEncryption};
use crate::util::{civil_local_to_unix, collect_sources, decode_legacy_name, normalize_entry_path, unix_to_local_civil, OutputPlanner, SourceItem};
use crate::volume::{open_reader, ReadSeek};
use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, ZipArchive, ZipWriter};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

type Archive = ZipArchive<Box<dyn ReadSeek>>;

fn open(path: &Path) -> Result<(Archive, u64, usize)> {
    let (r, size, vols) = open_reader(path)?;
    Ok((ZipArchive::new(r)?, size, vols))
}

fn dos_attr_from(external: u32, unix_mode: Option<u32>, is_dir: bool) -> u32 {
    if let Some(mode) = unix_mode {
        let mut a = if is_dir { 0x10 } else { 0x20 };
        if mode & 0o200 == 0 {
            a |= 0x01;
        }
        a
    } else {
        external & 0xFF
    }
}

fn zip_time_to_unix(dt: ::zip::DateTime) -> Option<i64> {
    civil_local_to_unix(dt.year() as i32, dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second())
}

fn entries(za: &Archive) -> Result<Vec<Entry>> {
    let meta = za.metadata();
    let mut out = Vec::with_capacity(meta.len());
    for i in 0..meta.len() {
        let e = meta.entry(i)?;
        let raw = e.name_raw();
        let path = normalize_entry_path(&decode_legacy_name(raw));
        if path.is_empty() {
            continue;
        }
        let is_dir = e.is_dir();
        out.push(Entry {
            path,
            is_dir,
            size: if is_dir { 0 } else { e.size() },
            packed_size: if is_dir { Some(0) } else { Some(e.compressed_size()) },
            modified: e.last_modified().and_then(zip_time_to_unix),
            crc32: if is_dir { None } else { Some(e.crc32()) },
            encrypted: e.encrypted(),
            attributes: Some(dos_attr_from(e.external_attributes(), e.unix_mode(), is_dir)),
            method: Some(method_name(e.compression())),
            index: i,
        });
    }
    Ok(out)
}

fn method_name(m: CompressionMethod) -> String {
    match m {
        CompressionMethod::Stored => "Store".into(),
        CompressionMethod::Deflated => "Deflate".into(),
        other => format!("{other:?}"),
    }
}

pub(crate) fn list(path: &Path, _opts: &OpenOptions, _p: &Progress) -> Result<ArchiveInfo> {
    let (za, size, vols) = open(path)?;
    let entries = entries(&za)?;
    Ok(ArchiveInfo {
        path: path.to_path_buf(),
        format: ArchiveFormat::Zip,
        encrypted: entries.iter().any(|e| e.encrypted),
        entries,
        solid: false,
        volumes: vols,
        archive_size: size,
    })
}

pub(crate) fn extract(path: &Path, dest: &Path, opts: &ExtractOptions, p: &Progress) -> Result<OpStats> {
    let (mut za, _, _) = open(path)?;
    let list = entries(&za)?;
    let planner = OutputPlanner::new(dest, opts);
    let chosen: Vec<&Entry> = list.iter().filter(|e| planner.wants(&e.path)).collect();
    p.set_totals(chosen.iter().map(|e| e.size).sum(), chosen.len() as u64);
    let mut stats = OpStats::default();
    for e in chosen {
        p.check()?;
        p.begin_file(&e.path);
        if e.is_dir {
            make_dir(&planner, &e.path, e.modified, &mut stats)?;
        } else {
            let mut f = match (&opts.password, e.encrypted) {
                (Some(pw), true) => za.by_index_decrypt(e.index, pw.as_bytes())?,
                (None, true) => return Err(Error::PasswordRequired),
                _ => za.by_index(e.index)?,
            };
            write_entry(&planner, &e.path, &mut f, Some(e.size), e.modified, opts.preserve_mtime, p, &mut stats)
                .map_err(|err| wrong_password_hint(err, e.encrypted))?;
        }
        p.file_done();
    }
    p.finish();
    Ok(stats)
}

/// A CRC failure on an encrypted entry almost always means a wrong password
/// (ZipCrypto's check byte lets 1/256 wrong passwords through).
fn wrong_password_hint(err: Error, encrypted: bool) -> Error {
    match err {
        Error::Io(ref io) if encrypted && io.to_string().to_lowercase().contains("checksum") => Error::WrongPassword,
        other => other,
    }
}

pub(crate) fn test(path: &Path, password: Option<&str>, p: &Progress) -> Result<TestReport> {
    let (mut za, _, _) = open(path)?;
    let list = entries(&za)?;
    let files: Vec<&Entry> = list.iter().filter(|e| !e.is_dir).collect();
    p.set_totals(files.iter().map(|e| e.size).sum(), files.len() as u64);
    let mut report = TestReport::default();
    for e in files {
        p.check()?;
        p.begin_file(&e.path);
        let res: Result<u64> = (|| {
            let mut f = match (password, e.encrypted) {
                (Some(pw), true) => za.by_index_decrypt(e.index, pw.as_bytes())?,
                (None, true) => return Err(Error::PasswordRequired),
                _ => za.by_index(e.index)?,
            };
            Ok(copy_progress(&mut f, &mut io::sink(), p)?)
        })();
        match res {
            Ok(n) => {
                report.files_tested += 1;
                report.bytes_tested += n;
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(Error::PasswordRequired) => return Err(Error::PasswordRequired),
            Err(Error::WrongPassword) => return Err(Error::WrongPassword),
            Err(err) => {
                let err = wrong_password_hint(err, e.encrypted);
                if matches!(err, Error::WrongPassword) {
                    return Err(err);
                }
                report.errors.push((e.path.clone(), err.to_string()));
            }
        }
        p.file_done();
    }
    p.finish();
    Ok(report)
}

fn file_options<'k>(item: &SourceItem, opts: &'k CreateOptions) -> Result<::zip::write::FileOptions<'k, 'static, ()>> {
    let mut o = SimpleFileOptions::default()
        .large_file(item.size >= 0xFFFF_FFFF)
        .unix_permissions(if item.is_dir { 0o755 } else { 0o644 });
    o = if opts.level == 0 || item.is_dir {
        o.compression_method(CompressionMethod::Stored)
    } else {
        o.compression_method(CompressionMethod::Deflated).compression_level(Some(opts.level.min(9) as i64))
    };
    if let Some(dt) = item.modified.and_then(unix_to_local_civil).and_then(|c| ::zip::DateTime::try_from(c).ok()) {
        o = o.last_modified_time(dt);
    }
    let o: ::zip::write::FileOptions<'k, 'static, ()> = match (&opts.password, item.is_dir) {
        (Some(pw), false) if !pw.is_empty() => match opts.zip_encryption {
            ZipEncryption::Aes256 => o.with_aes_encryption(::zip::AesMode::Aes256, pw.as_str()),
            ZipEncryption::ZipCrypto => {
                use ::zip::unstable::write::FileOptionsExt;
                o.with_deprecated_encryption(pw.as_bytes())?
            }
        },
        _ => o,
    };
    Ok(o)
}

fn write_items<W: Write + io::Seek>(zw: &mut ZipWriter<W>, items: &[SourceItem], opts: &CreateOptions, p: &Progress) -> Result<()> {
    for item in items {
        p.check()?;
        p.begin_file(&item.name);
        let o = file_options(item, opts)?;
        if item.is_dir {
            zw.add_directory(format!("{}/", item.name), o)?;
        } else {
            zw.start_file(item.name.clone(), o)?;
            let mut f = File::open(&item.fs_path)?;
            copy_progress(&mut f, zw, p)?;
        }
        p.file_done();
    }
    Ok(())
}

pub(crate) fn create(items: &[SourceItem], target: &Path, opts: &CreateOptions, p: &Progress) -> Result<()> {
    let file = File::create(target)?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(256 * 1024, file));
    write_items(&mut zw, items, opts, p)?;
    let mut w = zw.finish()?;
    w.flush()?;
    Ok(())
}

/// Rewrite the archive: raw-copy kept entries (no recompression, encryption
/// preserved), drop deleted/replaced ones, append new files.
pub(crate) fn modify(
    path: &Path,
    add: &[std::path::PathBuf],
    add_into: &str,
    delete: &[String],
    opts: &CreateOptions,
    p: &Progress,
) -> Result<()> {
    let new_items = collect_sources(add, add_into, &[path.to_path_buf()])?;
    let new_names: std::collections::HashSet<String> = new_items.iter().map(|i| i.name.clone()).collect();
    let src_file = File::open(path)?;
    let mut za = ZipArchive::new(io::BufReader::new(src_file))?;
    let tmp = path.with_file_name(format!(
        ".{}.okzip-tmp",
        path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    ));
    let result = (|| -> Result<()> {
        let mut zw = ZipWriter::new(BufWriter::new(File::create(&tmp)?));
        let sel = Some(delete.to_vec());
        let total_new: u64 = new_items.iter().map(|i| i.size).sum();
        p.set_totals(total_new, (za.len() + new_items.len()) as u64);
        for i in 0..za.len() {
            p.check()?;
            let f = za.by_index_raw(i)?;
            let name = normalize_entry_path(&decode_legacy_name(f.name_raw()));
            let deleted = !delete.is_empty() && crate::util::is_selected(&name, &sel);
            if deleted || new_names.contains(&name) {
                continue;
            }
            p.begin_file(&name);
            zw.raw_copy_file(f)?;
            p.file_done();
        }
        write_items(&mut zw, &new_items, opts, p)?;
        zw.finish()?.flush()?;
        Ok(())
    })();
    drop(za);
    match result {
        Ok(()) => {
            fs::rename(&tmp, path)?;
            p.finish();
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}
