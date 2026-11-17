//! Win32 front end.
//!
//! Command line (also what the Explorer context-menu verbs call):
//!
//! ```text
//! okzip.exe                           open the main window
//! okzip.exe <archive>                 open an archive
//! okzip.exe x  <archive>              extract here (next to the archive)
//! okzip.exe xs <archive>              extract to "<name>\" (smart: no double folder)
//! okzip.exe t  <archive>              test the archive
//! okzip.exe a [--format zip|7z|...] [--quick] [--gather] [--out FILE] <files/folders...>
//!                                     create ONE archive from all paths (dialog, or directly
//!                                     with --quick). --gather: Explorer starts one process per
//!                                     selected item; the first collects the others' paths
//!                                     (see `gather`) so a multi-selection makes one archive.
//! okzip.exe --register-shell | --unregister-shell | --associate | --unassociate
//!                                     silent Explorer integration (used by the installer)
//! ```

mod band;
mod dialogs;
mod dragout;
mod gather;
mod i18n;
mod icons_gen;
mod ids;
mod mainwnd;
mod progress;
mod registry;
mod settings;
mod theme;
mod util;

use i18n::{tr, trf, S};
use okzip_core::*;
use progress::{conflict_prompt, report_error, run_task, with_password};
use std::path::{Path, PathBuf};
use util::*;
use windows_sys::Win32::System::Ole::OleInitialize;
use windows_sys::Win32::UI::Controls::{InitCommonControlsEx, ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, ICC_TREEVIEW_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn run() {
    unsafe {
        // The manifest already declares PerMonitorV2; this covers unmanifested debug runs.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE (not just COM): drag-out uses DoDragDrop.
        OleInitialize(std::ptr::null());
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES | ICC_BAR_CLASSES | ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES | ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&icc);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("x") | Some("xs") | Some("t") if args.len() >= 2 => {
            let mut status = 0;
            for p in &args[1..] {
                let ok = match args[0].as_str() {
                    "x" => cli_extract(Path::new(p), false),
                    "xs" => cli_extract(Path::new(p), true),
                    _ => cli_test(Path::new(p)),
                };
                if !ok {
                    status = 1;
                }
            }
            status
        }
        Some("a") => cli_create(&args[1..]),
        Some(flag @ ("--register-shell" | "--unregister-shell" | "--associate" | "--unassociate")) => {
            let r = match flag {
                "--register-shell" => registry::register_context_menu(),
                "--unregister-shell" => registry::unregister_context_menu(),
                "--associate" => registry::associate(),
                _ => registry::unassociate(),
            };
            i32::from(r.is_err())
        }
        Some(path) => {
            let p = PathBuf::from(path);
            message_loop(Some(p))
        }
        None => message_loop(None),
    };
    std::process::exit(code);
}

fn message_loop(initial: Option<PathBuf>) -> i32 {
    unsafe {
        let hwnd = mainwnd::create_main_window(initial);
        let accel = mainwnd::accelerators();
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if TranslateAcceleratorW(hwnd, accel, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DestroyAcceleratorTable(accel);
        msg.wParam as i32
    }
}

/// "Extract here" / "Extract to <name>\" without opening the main window.
fn cli_extract(archive: &Path, subfolder: bool) -> bool {
    let owner = std::ptr::null_mut();
    let parent = archive.parent().map(Path::to_path_buf).unwrap_or_default();
    let cfg = settings::get();
    let mut pw: Option<String> = None;
    let dest = if subfolder {
        // Listing may itself need a password (encrypted headers).
        let a = archive.to_path_buf();
        let info = with_password(owner, archive, &mut pw, |pw| {
            let a = a.clone();
            run_task(owner, tr(S::GOpening), move |p, _| list(&a, &OpenOptions { password: pw }, &p))
        });
        match info {
            Ok(info) => smart_extract_dir(&info, &parent),
            Err(e) => {
                report_error(owner, &archive.display().to_string(), &e);
                return false;
            }
        }
    } else {
        parent
    };
    let a = archive.to_path_buf();
    let d = dest.clone();
    let result = with_password(owner, archive, &mut pw, |pw| {
        let (a, d) = (a.clone(), d.clone());
        run_task(owner, tr(S::GExtracting), move |p, dlg| {
            let mut o = ExtractOptions::new().password(pw).overwrite(cfg.overwrite.mode());
            o.on_conflict = Some(conflict_prompt(dlg));
            extract(&a, &d, &o, &p)
        })
    });
    match result {
        Ok(_) => {
            if cfg.open_folder_after_extract {
                util::shell_open(owner, &dest);
            }
            true
        }
        Err(e) => {
            report_error(owner, &archive.display().to_string(), &e);
            false
        }
    }
}

fn cli_test(archive: &Path) -> bool {
    let owner = std::ptr::null_mut();
    let a = archive.to_path_buf();
    let mut pw = None;
    let r = with_password(owner, archive, &mut pw, |pw| {
        let a = a.clone();
        run_task(owner, tr(S::GTesting), move |p, _| okzip_core::test(&a, pw.as_deref(), &p))
    });
    match r {
        Ok(r) if r.ok() => {
            info_box(owner, &trf(S::MTestOk, &[("n", &r.files_tested.to_string()), ("size", &format_size(r.bytes_tested))]));
            true
        }
        Ok(r) => {
            let list: Vec<String> = r.errors.iter().take(20).map(|(n, m)| format!("• {n}: {m}")).collect();
            error_box(owner, &trf(S::MTestFail, &[("n", &r.errors.len().to_string()), ("list", &list.join("\n"))]));
            false
        }
        Err(e) => {
            report_error(owner, &archive.display().to_string(), &e);
            false
        }
    }
}

/// `a [--format F] [--quick] [--out FILE] paths...`
fn cli_create(args: &[String]) -> i32 {
    let owner = std::ptr::null_mut();
    let cfg = settings::get();
    let mut format = cfg.format();
    let mut quick = false;
    let mut gather_mode = false;
    let mut out: Option<PathBuf> = None;
    let mut sources = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--format" | "-f" => {
                if let Some(f) = it.next().and_then(|v| ArchiveFormat::parse(v)) {
                    format = f;
                }
            }
            "--quick" | "-q" => quick = true,
            "--gather" => gather_mode = true,
            "--out" | "-o" => out = it.next().map(PathBuf::from),
            _ => sources.push(PathBuf::from(a)),
        }
    }
    if gather_mode {
        match gather::collect(&format!("a-{}", format.extension()), &sources) {
            Some(all) => sources = all,
            None => return 0, // handed our paths to the collecting process
        }
    }
    if sources.is_empty() {
        return message_loop(None);
    }
    if !quick {
        return match mainwnd::create_archive_interactive(owner, &sources, format) {
            Some(_) => 0,
            None => 1,
        };
    }
    let target = out.or_else(|| default_archive_path(&sources, format)).unwrap_or_else(|| PathBuf::from(format!("archive.{}", format.extension())));
    let target = if target.exists() { unique_path(&target) } else { target };
    let mut opts = CreateOptions::new(format);
    opts.level = cfg.level();
    let t2 = target.clone();
    match run_task(owner, tr(S::GCompressing), move |p, _| create(&sources, &t2, &opts, &p)) {
        Ok(_) => 0,
        Err(e) => {
            report_error(owner, &target.display().to_string(), &e);
            1
        }
    }
}
