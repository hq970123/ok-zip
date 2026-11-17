//! Multi-selection aggregation for Explorer verbs.
//!
//! Explorer runs a static context-menu command once per selected item, so
//! "压缩为 ZIP" on 5 files starts 5 `okzip a --gather` processes at once. The
//! first one to take a named mutex becomes the collector; the others drop a
//! small request file with their paths into a per-user spool folder and wait
//! until the collector has consumed it. The collector keeps picking up request
//! files until none has arrived for a short quiet period, then compresses
//! everything into one archive.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

use super::util::wide;

/// No new request for this long → the selection is complete.
const QUIET: Duration = Duration::from_millis(900);
/// Hard cap for collecting (very large selections still finish).
const MAX_COLLECT: Duration = Duration::from_secs(20);
/// How long a sender waits for the collector to take its request.
const SENDER_WAIT: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(40);

struct Mutex(HANDLE);

impl Mutex {
    fn open(name: &str) -> Option<Mutex> {
        let w = wide(name);
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, w.as_ptr()) };
        if h.is_null() {
            None
        } else {
            Some(Mutex(h))
        }
    }
    fn try_lock(&self) -> bool {
        let r = unsafe { WaitForSingleObject(self.0, 0) };
        r == WAIT_OBJECT_0 || r == WAIT_ABANDONED
    }
    fn unlock(&self) {
        unsafe { ReleaseMutex(self.0) };
    }
}

impl Drop for Mutex {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn spool_dir(key: &str) -> PathBuf {
    std::env::temp_dir().join(format!("OKZip-gather-{key}"))
}

fn read_request(p: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(p).map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
}

/// Take every finished request file in `dir` (deleting it) and append its paths.
fn drain(dir: &Path, into: &mut Vec<PathBuf>) -> bool {
    let mut got = false;
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "req")).collect();
    files.sort();
    for f in files {
        let paths = read_request(&f);
        if std::fs::remove_file(&f).is_ok() {
            for p in paths {
                if !into.contains(&p) {
                    into.push(p);
                }
            }
            got = true;
        }
    }
    got
}

/// Returns `Some(all paths)` in the collecting process, `None` in a process
/// whose paths were handed to the collector (it should just exit).
pub fn collect(key: &str, own: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let dir = spool_dir(key);
    let _ = std::fs::create_dir_all(&dir);
    let Some(mutex) = Mutex::open(&format!("Local\\OKZip-gather-{key}")) else { return Some(own.to_vec()) };

    if !mutex.try_lock() {
        // Sender: write "<pid>.tmp" then rename to ".req" so the collector never
        // reads a half-written file.
        let name = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let tmp = dir.join(format!("{name}.tmp"));
        let req = dir.join(format!("{name}.req"));
        let body: String = own.iter().map(|p| format!("{}\n", p.display())).collect();
        if std::fs::write(&tmp, body).is_err() || std::fs::rename(&tmp, &req).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return Some(own.to_vec());
        }
        let start = Instant::now();
        loop {
            if !req.exists() {
                return None; // the collector took it
            }
            // The collector finished without seeing us: take over (our own
            // request is still in the spool and gets drained below).
            if mutex.try_lock() {
                break;
            }
            if start.elapsed() > SENDER_WAIT {
                let _ = std::fs::remove_file(&req);
                if req.exists() {
                    return None;
                }
                return Some(own.to_vec());
            }
            std::thread::sleep(POLL);
        }
    }

    // Collector.
    let mut all: Vec<PathBuf> = own.to_vec();
    let start = Instant::now();
    let mut last = Instant::now();
    loop {
        if drain(&dir, &mut all) {
            last = Instant::now();
        }
        if last.elapsed() >= QUIET || start.elapsed() >= MAX_COLLECT {
            break;
        }
        std::thread::sleep(POLL);
    }
    mutex.unlock();
    // Anything that slipped in right before unlocking.
    drain(&dir, &mut all);
    all.retain(|p| p.exists());
    Some(all)
}
