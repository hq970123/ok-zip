//! Per-format backends.

pub(crate) mod sevenz;
pub(crate) mod tar;
pub(crate) mod zip;

#[cfg(feature = "rar")]
pub(crate) mod rar;

use crate::error::Result;
use crate::progress::{copy_progress, Progress};
use crate::types::OpStats;
use crate::util::{set_mtime, OutputPlanner, Planned};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};

/// Write one entry's data to disk according to the planner's policy.
pub(crate) fn write_entry<R: Read + ?Sized>(
    planner: &OutputPlanner<'_>,
    entry_path: &str,
    data: &mut R,
    mtime: Option<i64>,
    preserve_mtime: bool,
    progress: &Progress,
    stats: &mut OpStats,
) -> Result<()> {
    match planner.file_path(entry_path)? {
        Planned::Skip => {
            stats.skipped += 1;
            // Still consume nothing: callers that must drain do so themselves.
            Ok(())
        }
        Planned::Write(target) => {
            let result = (|| -> Result<u64> {
                let mut w = BufWriter::with_capacity(256 * 1024, File::create(&target)?);
                let n = copy_progress(data, &mut w, progress)?;
                w.flush()?;
                Ok(n)
            })();
            match result {
                Ok(n) => {
                    if preserve_mtime {
                        set_mtime(&target, mtime);
                    }
                    stats.files += 1;
                    stats.bytes += n;
                    stats.outputs.push(target);
                    Ok(())
                }
                Err(e) => {
                    // Don't leave half-written files behind.
                    let _ = fs::remove_file(&target);
                    Err(e)
                }
            }
        }
    }
}

/// Create the directory for a directory entry.
pub(crate) fn make_dir(
    planner: &OutputPlanner<'_>,
    entry_path: &str,
    mtime: Option<i64>,
    stats: &mut OpStats,
) -> Result<()> {
    if let Some(dir) = planner.dir_path(entry_path) {
        if dir.is_file() {
            return Ok(());
        }
        fs::create_dir_all(&dir)?;
        let _ = mtime; // directory mtimes get bumped by later writes; skip
        stats.dirs += 1;
    }
    Ok(())
}
