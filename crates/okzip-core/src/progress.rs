//! Progress reporting and cooperative cancellation.

use crate::error::{cancelled_io, Error, Result};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A snapshot of how far an operation has come.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgressInfo {
    /// Entry currently being processed (archive-relative path).
    pub current: String,
    /// Bytes processed so far (uncompressed bytes for extract/create, raw
    /// archive bytes for streaming listings).
    pub bytes_done: u64,
    /// Expected total; 0 when unknown.
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
}

impl ProgressInfo {
    /// Completion in the range 0.0..=1.0 (bytes when known, else files).
    pub fn fraction(&self) -> f64 {
        if self.bytes_total > 0 {
            (self.bytes_done as f64 / self.bytes_total as f64).clamp(0.0, 1.0)
        } else if self.files_total > 0 {
            (self.files_done as f64 / self.files_total as f64).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Callback invoked (throttled) whenever progress changes.
pub type ProgressCallback = dyn Fn(&ProgressInfo) + Send + Sync;

struct Inner {
    info: ProgressInfo,
    last_emit: Option<Instant>,
}

/// Progress sink + cancellation token. Cheap to clone; all clones share state.
#[derive(Clone)]
pub struct Progress {
    cancel: Arc<AtomicBool>,
    callback: Option<Arc<ProgressCallback>>,
    inner: Arc<Mutex<Inner>>,
    interval: Duration,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl Progress {
    /// A progress sink that reports nothing (cancellation still works).
    pub fn new() -> Self {
        Progress {
            cancel: Arc::new(AtomicBool::new(false)),
            callback: None,
            inner: Arc::new(Mutex::new(Inner { info: ProgressInfo::default(), last_emit: None })),
            interval: Duration::from_millis(40),
        }
    }

    /// A progress sink that calls `f` at most every ~40 ms.
    pub fn with_callback<F>(f: F) -> Self
    where
        F: Fn(&ProgressInfo) + Send + Sync + 'static,
    {
        let mut p = Self::new();
        p.callback = Some(Arc::new(f));
        p
    }

    /// Use an externally owned cancellation flag (e.g. a GUI "Cancel" button).
    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = flag;
        self
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Returns `Err(Cancelled)` once cancellation was requested.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn snapshot(&self) -> ProgressInfo {
        self.inner.lock().map(|g| g.info.clone()).unwrap_or_default()
    }

    fn update(&self, force: bool, f: impl FnOnce(&mut ProgressInfo)) {
        let Ok(mut g) = self.inner.lock() else { return };
        f(&mut g.info);
        let Some(cb) = &self.callback else { return };
        let now = Instant::now();
        let due = force || g.last_emit.is_none_or(|t| now.duration_since(t) >= self.interval);
        if due {
            g.last_emit = Some(now);
            let snap = g.info.clone();
            drop(g);
            cb(&snap);
        }
    }

    pub(crate) fn set_totals(&self, bytes_total: u64, files_total: u64) {
        self.update(true, |i| {
            i.bytes_total = bytes_total;
            i.files_total = files_total;
            i.bytes_done = 0;
            i.files_done = 0;
        });
    }

    pub(crate) fn begin_file(&self, name: &str) {
        self.update(false, |i| i.current = name.to_string());
    }

    pub(crate) fn file_done(&self) {
        self.update(false, |i| i.files_done += 1);
    }

    pub(crate) fn add_bytes(&self, n: u64) {
        self.update(false, |i| i.bytes_done += n);
    }

    pub(crate) fn set_bytes(&self, n: u64) {
        self.update(false, |i| i.bytes_done = n);
    }

    /// Emit a final, unthrottled snapshot.
    pub(crate) fn finish(&self) {
        self.update(true, |i| {
            if i.bytes_total > 0 {
                i.bytes_done = i.bytes_total;
            }
            if i.files_total > 0 {
                i.files_done = i.files_total;
            }
        });
    }
}

/// Copy with progress + cancellation checks between 256 KiB chunks.
pub(crate) fn copy_progress<R: Read + ?Sized, W: Write + ?Sized>(
    r: &mut R,
    w: &mut W,
    p: &Progress,
) -> io::Result<u64> {
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        if p.is_cancelled() {
            return Err(cancelled_io());
        }
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        w.write_all(&buf[..n])?;
        total += n as u64;
        p.add_bytes(n as u64);
    }
    Ok(total)
}

/// Reader adapter that reports bytes read and aborts on cancellation.
pub(crate) struct ProgressReader<R> {
    inner: R,
    progress: Progress,
}

impl<R> ProgressReader<R> {
    pub(crate) fn new(inner: R, progress: &Progress) -> Self {
        ProgressReader { inner, progress: progress.clone() }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(cancelled_io());
        }
        let n = self.inner.read(buf)?;
        self.progress.add_bytes(n as u64);
        Ok(n)
    }
}

/// Reader adapter that reports the absolute position of the underlying raw
/// stream (used for streaming formats where only the compressed size is known).
pub(crate) struct PositionReader<R> {
    inner: R,
    pos: u64,
    progress: Progress,
}

impl<R> PositionReader<R> {
    pub(crate) fn new(inner: R, progress: &Progress) -> Self {
        PositionReader { inner, pos: 0, progress: progress.clone() }
    }
}

impl<R: Read> Read for PositionReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(cancelled_io());
        }
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        self.progress.set_bytes(self.pos);
        Ok(n)
    }
}
