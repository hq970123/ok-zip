//! Format-independent entry points.

use crate::detect::detect_format;
use crate::error::{Error, Result};
use crate::formats;
use crate::progress::Progress;
use crate::types::*;
use crate::util::{collect_sources, copy_into, is_selected, sanitize_rel_path, TempDir};
use crate::volume::split_file;
use std::fs;
use std::path::{Path, PathBuf};

/// List an archive's contents.
pub fn list(path: &Path, opts: &OpenOptions, p: &Progress) -> Result<ArchiveInfo> {
    let fmt = detect_format(path)?;
    match fmt {
        ArchiveFormat::Zip => formats::zip::list(path, opts, p),
        ArchiveFormat::SevenZ => formats::sevenz::list(path, opts, p),
        #[cfg(feature = "rar")]
        ArchiveFormat::Rar => formats::rar::list(path, opts, p),
        #[cfg(not(feature = "rar"))]
        ArchiveFormat::Rar => Err(Error::Unsupported("RAR support was not compiled in".into())),
        f => formats::tar::list(path, f, opts, p),
    }
}

/// Extract (all or a selection of) an archive into `dest`.
pub fn extract(path: &Path, dest: &Path, opts: &ExtractOptions, p: &Progress) -> Result<OpStats> {
    fs::create_dir_all(dest)?;
    let fmt = detect_format(path)?;
    match fmt {
        ArchiveFormat::Zip => formats::zip::extract(path, dest, opts, p),
        ArchiveFormat::SevenZ => formats::sevenz::extract(path, dest, opts, p),
        #[cfg(feature = "rar")]
        ArchiveFormat::Rar => formats::rar::extract(path, dest, opts, p),
        #[cfg(not(feature = "rar"))]
        ArchiveFormat::Rar => Err(Error::Unsupported("RAR support was not compiled in".into())),
        f => formats::tar::extract(path, f, dest, opts, p),
    }
}

/// Verify every entry decompresses (and matches its CRC where the format has one).
pub fn test(path: &Path, password: Option<&str>, p: &Progress) -> Result<TestReport> {
    let fmt = detect_format(path)?;
    match fmt {
        ArchiveFormat::Zip => formats::zip::test(path, password, p),
        ArchiveFormat::SevenZ => formats::sevenz::test(path, password, p),
        #[cfg(feature = "rar")]
        ArchiveFormat::Rar => formats::rar::test(path, password, p),
        #[cfg(not(feature = "rar"))]
        ArchiveFormat::Rar => Err(Error::Unsupported("RAR support was not compiled in".into())),
        f => formats::tar::test(path, f, p),
    }
}

/// Create a new archive at `target` from files/folders on disk.
///
/// With `split_size`, the archive is written then cut into
/// `target.001`, `target.002`, ... (7-Zip/Bandizip compatible raw split).
pub fn create(sources: &[PathBuf], target: &Path, opts: &CreateOptions, p: &Progress) -> Result<OpStats> {
    if !opts.format.is_writable() {
        return Err(Error::Unsupported(format!("creating {} archives", opts.format)));
    }
    if opts.password.as_deref().is_some_and(|s| !s.is_empty()) && !opts.format.supports_password() {
        return Err(Error::Unsupported(format!("{} archives cannot be password protected", opts.format)));
    }
    if sources.is_empty() {
        return Err(Error::Other("nothing to compress".into()));
    }
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let items = collect_sources(sources, "", &[target.to_path_buf()])?;
    p.set_totals(items.iter().map(|i| i.size).sum(), items.len() as u64);

    let split = opts.split_size.filter(|s| *s > 0);
    let write_to = match split {
        Some(_) => target.with_file_name(format!(
            ".{}.okzip-part",
            target.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
        )),
        None => target.to_path_buf(),
    };
    let res = match opts.format {
        ArchiveFormat::Zip => formats::zip::create(&items, &write_to, opts, p),
        ArchiveFormat::SevenZ => formats::sevenz::create(&items, &write_to, opts, p),
        _ => formats::tar::create(&items, &write_to, opts, p),
    };
    if let Err(e) = res {
        let _ = fs::remove_file(&write_to);
        return Err(e);
    }
    let outputs = match split {
        Some(size) => match split_file(&write_to, target, size, p) {
            Ok(v) => v,
            Err(e) => {
                let _ = fs::remove_file(&write_to);
                return Err(e);
            }
        },
        None => vec![target.to_path_buf()],
    };
    p.finish();
    Ok(OpStats {
        files: items.iter().filter(|i| !i.is_dir).count() as u64,
        dirs: items.iter().filter(|i| i.is_dir).count() as u64,
        bytes: items.iter().map(|i| i.size).sum(),
        skipped: 0,
        outputs,
    })
}

/// Add files to / delete entries from an existing archive.
///
/// ZIP is rewritten entry-by-entry (kept entries are raw-copied, so their
/// compression and encryption are untouched). 7z and TAR-family archives are
/// unpacked to a temporary folder and rebuilt with `opts`. RAR, bare streams
/// and split volumes are read-only.
pub fn modify(path: &Path, ops: &ModifyOps, opts: &CreateOptions, p: &Progress) -> Result<()> {
    let info = list(path, &OpenOptions { password: opts.password.clone() }, p)?;
    if info.volumes > 1 || crate::volume::volume_set(path).is_some() {
        return Err(Error::Unsupported("modifying split archives".into()));
    }
    match info.format {
        ArchiveFormat::Zip => formats::zip::modify(path, &ops.add, &ops.add_into, &ops.delete, opts, p),
        ArchiveFormat::SevenZ | ArchiveFormat::Tar | ArchiveFormat::TarGz | ArchiveFormat::TarBz2 | ArchiveFormat::TarXz | ArchiveFormat::TarZst => {
            rebuild(path, &info, ops, opts, p)
        }
        f => Err(Error::Unsupported(format!("modifying {f} archives"))),
    }
}

fn rebuild(path: &Path, info: &ArchiveInfo, ops: &ModifyOps, opts: &CreateOptions, p: &Progress) -> Result<()> {
    let tmp = TempDir::new_near(path)?;
    let stage = tmp.0.join("stage");
    fs::create_dir_all(&stage)?;
    let mut xo = ExtractOptions::new().password(opts.password.clone());
    xo.overwrite = OverwriteMode::Overwrite;
    extract(path, &stage, &xo, p)?;
    // Delete
    if !ops.delete.is_empty() {
        let sel = Some(ops.delete.clone());
        let mut doomed: Vec<&Entry> = info.entries.iter().filter(|e| is_selected(&e.path, &sel)).collect();
        doomed.sort_by_key(|e| std::cmp::Reverse(e.path.len()));
        for e in doomed {
            if let Some(rel) = sanitize_rel_path(&e.path) {
                let target = stage.join(rel);
                if target.is_dir() {
                    let _ = fs::remove_dir_all(&target);
                } else {
                    let _ = fs::remove_file(&target);
                }
            }
        }
    }
    // Add
    let into = match sanitize_rel_path(&ops.add_into) {
        Some(rel) => stage.join(rel),
        None => stage.clone(),
    };
    fs::create_dir_all(&into)?;
    for src in &ops.add {
        p.check()?;
        copy_into(src, &into)?;
    }
    // Rebuild with the archive's own format.
    let mut co = opts.clone();
    co.format = info.format;
    co.split_size = None;
    let mut roots: Vec<PathBuf> = fs::read_dir(&stage)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    roots.sort();
    let out = tmp.0.join("rebuilt");
    if roots.is_empty() {
        return Err(Error::Unsupported("removing every entry (the archive would be empty)".into()));
    }
    create(&roots, &out, &co, p)?;
    fs::rename(&out, path).or_else(|_| fs::copy(&out, path).map(|_| ()))?;
    Ok(())
}
