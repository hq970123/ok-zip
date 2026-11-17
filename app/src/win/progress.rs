//! Runs archive operations on a worker thread behind a progress dialog with
//! a working Cancel button, pumping the UI message loop meanwhile.

use super::dialogs::{conflict_dialog, password_dialog, ConflictQuestion};
use super::i18n::{error_text, tr, trf, S};
use super::ids::*;
use super::util::*;
use okzip_core::{ConflictChoice, ConflictInfo, Error, Progress, ProgressInfo};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TASK_DONE: u32 = WM_APP + 41;
/// Sent (not posted) by the worker: lParam = *mut ConflictQuestion. The UI
/// thread shows the conflict dialog and writes the answer back while the
/// worker is blocked inside SendMessageW.
const WM_ASK_CONFLICT: u32 = WM_APP + 42;
const TIMER_SHOW: usize = 1;
const TIMER_TICK: usize = 2;

struct Shared {
    info: Mutex<ProgressInfo>,
    done: AtomicBool,
    cancel: Arc<AtomicBool>,
}

struct DlgCtx {
    shared: Arc<Shared>,
    title: String,
    op: String,
    start: Instant,
    show_immediately: bool,
}

unsafe extern "system" fn progress_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
                let ctx = &*(lp as *const DlgCtx);
                set_text(dlg, &ctx.title);
                set_dlg_text(dlg, IDC_G_OP, &ctx.op);
                set_dlg_text(dlg, IDCANCEL, tr(S::GCancel));
                SendDlgItemMessageW(dlg, IDC_G_BAR, PBM_SETRANGE32, 0, 1000);
                if ctx.show_immediately {
                    ShowWindow(dlg, SW_SHOW);
                } else {
                    SetTimer(dlg, TIMER_SHOW, 350, None);
                }
                SetTimer(dlg, TIMER_TICK, 100, None);
                1
            }
            WM_TIMER => {
                let ptr = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *const DlgCtx;
                if ptr.is_null() {
                    return 0;
                }
                let ctx = &*ptr;
                if wp == TIMER_SHOW {
                    KillTimer(dlg, TIMER_SHOW);
                    if !ctx.shared.done.load(Ordering::SeqCst) {
                        ShowWindow(dlg, SW_SHOW);
                    }
                } else if wp == TIMER_TICK {
                    let info = ctx.shared.info.lock().map(|g| g.clone()).unwrap_or_default();
                    refresh(dlg, ctx, &info);
                }
                1
            }
            WM_COMMAND => {
                if loword(wp) as i32 == IDCANCEL {
                    let ptr = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *const DlgCtx;
                    if !ptr.is_null() {
                        let c = &*ptr;
                        c.shared.cancel.store(true, Ordering::SeqCst);
                        set_dlg_text(dlg, IDC_G_OP, tr(S::GCancelling));
                        enable(dlg_item(dlg, IDCANCEL), false);
                    }
                }
                1
            }
            WM_CLOSE => {
                SendMessageW(dlg, WM_COMMAND, IDCANCEL as usize, 0);
                1
            }
            WM_ASK_CONFLICT => {
                let q = &mut *(lp as *mut ConflictQuestion);
                conflict_dialog(dlg, q);
                1
            }
            _ => 0,
        }
    }
}

fn refresh(dlg: HWND, ctx: &DlgCtx, info: &ProgressInfo) {
    let frac = info.fraction();
    unsafe { SendDlgItemMessageW(dlg, IDC_G_BAR, PBM_SETPOS, (frac * 1000.0) as usize, 0) };
    set_dlg_text(dlg, IDC_G_FILE, &info.current.replace('/', "\\"));
    let elapsed = ctx.start.elapsed().as_secs();
    let stats = if info.bytes_total > 0 {
        let remain = if frac > 0.01 { ((elapsed as f64) / frac - elapsed as f64).max(0.0) as u64 } else { 0 };
        trf(
            S::GStats,
            &[
                ("done", &okzip_core::format_size(info.bytes_done)),
                ("total", &okzip_core::format_size(info.bytes_total)),
                ("elapsed", &format_duration(elapsed)),
                ("remain", &if frac > 0.01 { format_duration(remain) } else { "--:--".into() }),
            ],
        )
    } else {
        trf(S::GStatsNoTotal, &[("done", &okzip_core::format_size(info.bytes_done)), ("elapsed", &format_duration(elapsed))])
    };
    set_dlg_text(dlg, IDC_G_STATS, &stats);
    let pct = (frac * 100.0) as u32;
    set_text(dlg, &format!("{pct}% - {}", ctx.title));
}

/// Run `work` on a worker thread while showing a progress dialog owned by
/// `owner` (which is disabled meanwhile). Returns the worker's result.
pub fn run_task<T, F>(owner: HWND, op: &str, work: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce(Progress, SendHwnd) -> Result<T, Error> + Send + 'static,
{
    let cancel = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Shared { info: Mutex::new(ProgressInfo::default()), done: AtomicBool::new(false), cancel: cancel.clone() });
    let ctx = Box::new(DlgCtx {
        shared: shared.clone(),
        title: tr(S::AppName).to_string(),
        op: op.to_string(),
        start: Instant::now(),
        show_immediately: owner.is_null(),
    });
    let dlg = create_dialog_res(owner, IDD_PROGRESS, Some(progress_proc), &*ctx as *const DlgCtx as isize);
    let dlg_send = SendHwnd::new(dlg);
    if !owner.is_null() {
        enable(owner, false);
    }

    let sh = shared.clone();
    let progress = Progress::with_callback(move |i: &ProgressInfo| {
        if let Ok(mut g) = sh.info.lock() {
            *g = i.clone();
        }
    })
    .with_cancel_flag(cancel.clone());

    let result: Arc<Mutex<Option<Result<T, Error>>>> = Arc::new(Mutex::new(None));
    let r2 = result.clone();
    let sh2 = shared.clone();
    let handle = std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(progress, dlg_send)))
            .unwrap_or_else(|_| Err(Error::Other("internal error (worker panicked)".into())));
        if let Ok(mut g) = r2.lock() {
            *g = Some(r);
        }
        sh2.done.store(true, Ordering::SeqCst);
        unsafe { PostMessageW(dlg_send.get(), WM_TASK_DONE, 0, 0) };
    });

    // Nested message loop until the worker finishes.
    let mut quit: Option<WPARAM> = None;
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while !shared.done.load(Ordering::SeqCst) {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r == 0 {
                quit = Some(msg.wParam);
                cancel.store(true, Ordering::SeqCst);
                continue;
            }
            if r == -1 {
                break;
            }
            if IsDialogMessageW(dlg, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    let _ = handle.join();
    unsafe {
        if !owner.is_null() {
            enable(owner, true);
            SetForegroundWindow(owner);
        }
        DestroyWindow(dlg);
        if let Some(code) = quit {
            PostQuitMessage(code as i32);
        }
    }
    drop(ctx);
    let out = result.lock().ok().and_then(|mut g| g.take());
    out.unwrap_or(Err(Error::Cancelled))
}

/// Retry `op` with a password prompt whenever it reports a missing or wrong
/// password. `password` is the session password; it's updated on success.
pub fn with_password<T, F>(owner: HWND, archive: &Path, password: &mut Option<String>, mut op: F) -> Result<T, Error>
where
    F: FnMut(Option<String>) -> Result<T, Error>,
{
    let name = archive.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut wrong = false;
    loop {
        match op(password.clone()) {
            Err(e) if e.needs_password() => {
                wrong = wrong || matches!(e, Error::WrongPassword) || password.is_some();
                let prompt = if wrong { trf(S::PWrong, &[("name", &name)]) } else { trf(S::PPrompt, &[("name", &name)]) };
                match password_dialog(owner, &prompt) {
                    Some(pw) => {
                        *password = Some(pw);
                        wrong = true;
                    }
                    None => return Err(Error::Cancelled),
                }
            }
            other => return other,
        }
    }
}

/// Conflict prompt used for OverwriteMode::Ask. Runs on the worker thread:
/// it blocks in SendMessageW while the progress dialog's (UI) thread shows the
/// overwrite dialog.
pub fn conflict_prompt(dlg: SendHwnd) -> Arc<okzip_core::ConflictCallback> {
    Arc::new(move |c: &ConflictInfo<'_>| {
        let meta = std::fs::metadata(c.target).ok();
        let old_modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        let mut q = ConflictQuestion {
            target: c.target.to_path_buf(),
            old_size: meta.as_ref().map(|m| m.len()),
            old_modified,
            new_size: c.new_size,
            new_modified: c.new_modified,
            answer: ConflictChoice::Cancel,
        };
        unsafe { SendMessageW(dlg.get(), WM_ASK_CONFLICT, 0, &mut q as *mut ConflictQuestion as isize) };
        q.answer
    })
}

/// Show a core error unless it is a user cancel.
pub fn report_error(owner: HWND, context: &str, e: &Error) {
    if matches!(e, Error::Cancelled) {
        return;
    }
    let msg = if context.is_empty() { error_text(e) } else { format!("{context}\n\n{}", error_text(e)) };
    error_box(owner, &msg);
}
