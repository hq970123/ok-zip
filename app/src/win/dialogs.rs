//! Modal dialogs (templates live in res/app.rc): password, extract, create,
//! settings. Captions are filled from the string tables at WM_INITDIALOG.

use super::i18n::{tr, trf, S};
use super::ids::*;
use super::registry;
use super::util::*;
use okzip_core::{ArchiveFormat, CreateOptions, OverwriteMode, ZipEncryption};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn ctx<'a, T>(dlg: HWND) -> Option<&'a mut T> {
    unsafe {
        let p = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut T;
        if p.is_null() {
            None
        } else {
            Some(&mut *p)
        }
    }
}

fn store<T>(dlg: HWND, lp: LPARAM) -> &'static mut T {
    unsafe {
        SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
        &mut *(lp as *mut T)
    }
}

fn run_modal<T>(owner: HWND, id: i32, state: &mut T, proc_: DLGPROC) -> bool {
    unsafe { DialogBoxParamW(hinstance(), make_int_resource(id), owner, proc_, state as *mut T as isize) == IDOK as isize }
}

fn combo_add(dlg: HWND, id: i32, text: &str) {
    let w = wide(text);
    unsafe { SendDlgItemMessageW(dlg, id, CB_ADDSTRING, 0, w.as_ptr() as isize) };
}

fn combo_sel(dlg: HWND, id: i32) -> i32 {
    unsafe { SendDlgItemMessageW(dlg, id, CB_GETCURSEL, 0, 0) as i32 }
}

fn combo_set(dlg: HWND, id: i32, idx: i32) {
    unsafe { SendDlgItemMessageW(dlg, id, CB_SETCURSEL, idx as usize, 0) };
}

fn set_password_visible(dlg: HWND, ids: &[i32], visible: bool) {
    for &id in ids {
        let h = dlg_item(dlg, id);
        unsafe {
            SendMessageW(h, EM_SETPASSWORDCHAR, if visible { 0 } else { 0x25CF }, 0);
            InvalidateRect(h, std::ptr::null(), 1);
        }
    }
}

use windows_sys::Win32::Graphics::Gdi::InvalidateRect;

// ---------------------------------------------------------------- password

struct PwState {
    prompt: String,
    result: String,
}

unsafe extern "system" fn pw_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let st: &mut PwState = store(dlg, lp);
            set_text(dlg, tr(S::PTitle));
            set_dlg_text(dlg, IDC_P_PROMPT, &st.prompt);
            set_dlg_text(dlg, IDC_P_SHOW, tr(S::PShow));
            set_dlg_text(dlg, IDOK, if super::i18n::is_zh() { "确定" } else { "OK" });
            set_dlg_text(dlg, IDCANCEL, tr(S::GCancel));
            set_password_visible(dlg, &[IDC_P_PW], false);
            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(dlg_item(dlg, IDC_P_PW)) };
            0
        }
        WM_COMMAND => {
            let id = loword(wp) as i32;
            match id {
                IDOK => {
                    if let Some(st) = ctx::<PwState>(dlg) {
                        st.result = get_dlg_text(dlg, IDC_P_PW);
                    }
                    unsafe { EndDialog(dlg, IDOK as isize) };
                }
                IDCANCEL => unsafe {
                    EndDialog(dlg, IDCANCEL as isize);
                },
                IDC_P_SHOW => set_password_visible(dlg, &[IDC_P_PW], is_checked(dlg, IDC_P_SHOW)),
                _ => {}
            }
            1
        }
        _ => 0,
    }
}

/// Ask for a password. `None` when cancelled.
pub fn password_dialog(owner: HWND, prompt: &str) -> Option<String> {
    let mut st = PwState { prompt: prompt.to_string(), result: String::new() };
    if run_modal(owner, IDD_PASSWORD, &mut st, Some(pw_proc)) {
        Some(st.result)
    } else {
        None
    }
}

// ---------------------------------------------------------------- extract

pub struct ExtractChoice {
    pub dest: PathBuf,
    pub overwrite: OverwriteMode,
    pub open_after: bool,
    pub password: Option<String>,
}

struct XState {
    base: PathBuf,
    stem: String,
    scope_text: String,
    selected_only: bool,
    password: Option<String>,
    out: Option<ExtractChoice>,
}

unsafe extern "system" fn extract_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let st: &mut XState = store(dlg, lp);
            set_text(dlg, tr(if st.selected_only { S::XTitleSel } else { S::XTitle }));
            set_dlg_text(dlg, IDC_X_DEST_LABEL, tr(S::XDest));
            set_dlg_text(dlg, IDC_X_BROWSE, tr(S::XBrowse));
            set_dlg_text(dlg, IDC_X_SUBFOLDER, &trf(S::XSubfolder, &[("name", &st.stem)]));
            set_dlg_text(dlg, IDC_X_OW_GROUP, tr(S::XOwGroup));
            set_dlg_text(dlg, IDC_X_OW_ASK, tr(S::XOwAsk));
            set_dlg_text(dlg, IDC_X_OW_OVERWRITE, tr(S::XOwOverwrite));
            set_dlg_text(dlg, IDC_X_OW_SKIP, tr(S::XOwSkip));
            set_dlg_text(dlg, IDC_X_OW_RENAME, tr(S::XOwRename));
            set_dlg_text(dlg, IDC_X_OTHER_GROUP, tr(S::XOther));
            set_dlg_text(dlg, IDC_X_OPEN_AFTER, tr(S::XOpenAfter));
            set_dlg_text(dlg, IDC_X_PW_LABEL, tr(S::XPassword));
            set_dlg_text(dlg, IDC_X_SCOPE, &st.scope_text);
            set_dlg_text(dlg, IDOK, if super::i18n::is_zh() { "解压" } else { "Extract" });
            set_dlg_text(dlg, IDCANCEL, tr(S::GCancel));
            set_checked(dlg, IDC_X_SUBFOLDER, true);
            set_dlg_text(dlg, IDC_X_DEST, &st.base.join(&st.stem).display().to_string());
            set_checked(dlg, IDC_X_OW_ASK, true);
            set_checked(dlg, IDC_X_OPEN_AFTER, true);
            if let Some(pw) = &st.password {
                set_dlg_text(dlg, IDC_X_PW, pw);
            }
            1
        }
        WM_COMMAND => {
            let id = loword(wp) as i32;
            let Some(st) = ctx::<XState>(dlg) else { return 0 };
            match id {
                IDC_X_BROWSE => {
                    let cur = PathBuf::from(get_dlg_text(dlg, IDC_X_DEST));
                    let start = if cur.is_dir() { cur } else { st.base.clone() };
                    if let Some(p) = browse_folder(dlg, tr(S::XPickFolder), &start) {
                        st.base = p.clone();
                        let shown = if is_checked(dlg, IDC_X_SUBFOLDER) { p.join(&st.stem) } else { p };
                        set_dlg_text(dlg, IDC_X_DEST, &shown.display().to_string());
                    }
                }
                IDC_X_SUBFOLDER => {
                    let cur = PathBuf::from(get_dlg_text(dlg, IDC_X_DEST));
                    let on = is_checked(dlg, IDC_X_SUBFOLDER);
                    let ends_with_stem = cur.file_name().is_some_and(|n| n.to_string_lossy() == st.stem);
                    let next = if on && !ends_with_stem {
                        cur.join(&st.stem)
                    } else if !on && ends_with_stem {
                        cur.parent().map(Path::to_path_buf).unwrap_or(cur)
                    } else {
                        cur
                    };
                    set_dlg_text(dlg, IDC_X_DEST, &next.display().to_string());
                }
                IDOK => {
                    let dest = PathBuf::from(get_dlg_text(dlg, IDC_X_DEST).trim());
                    if dest.as_os_str().is_empty() {
                        return 1;
                    }
                    let overwrite = if is_checked(dlg, IDC_X_OW_OVERWRITE) {
                        OverwriteMode::Overwrite
                    } else if is_checked(dlg, IDC_X_OW_SKIP) {
                        OverwriteMode::Skip
                    } else if is_checked(dlg, IDC_X_OW_RENAME) {
                        OverwriteMode::Rename
                    } else {
                        OverwriteMode::Ask
                    };
                    let pw = get_dlg_text(dlg, IDC_X_PW);
                    st.out = Some(ExtractChoice {
                        dest,
                        overwrite,
                        open_after: is_checked(dlg, IDC_X_OPEN_AFTER),
                        password: if pw.is_empty() { None } else { Some(pw) },
                    });
                    unsafe { EndDialog(dlg, IDOK as isize) };
                }
                IDCANCEL => unsafe {
                    EndDialog(dlg, IDCANCEL as isize);
                },
                _ => {}
            }
            1
        }
        _ => 0,
    }
}

pub fn extract_dialog(
    owner: HWND,
    base: &Path,
    stem: &str,
    scope_text: &str,
    selected_only: bool,
    password: Option<String>,
) -> Option<ExtractChoice> {
    let mut st = XState {
        base: base.to_path_buf(),
        stem: stem.to_string(),
        scope_text: scope_text.to_string(),
        selected_only,
        password,
        out: None,
    };
    run_modal(owner, IDD_EXTRACT, &mut st, Some(extract_proc));
    st.out
}

// ---------------------------------------------------------------- create

pub struct CreateChoice {
    pub target: PathBuf,
    pub options: CreateOptions,
}

struct CState {
    sources: Vec<PathBuf>,
    initial: PathBuf,
    format: ArchiveFormat,
    out: Option<CreateChoice>,
}

const LEVELS: [(u32, S); 6] = [(0, S::CLevel0), (1, S::CLevel1), (3, S::CLevel3), (5, S::CLevel5), (7, S::CLevel7), (9, S::CLevel9)];
const SPLITS: [&str; 5] = ["", "10 MB", "100 MB", "700 MB", "4092 MB"];

fn strip_archive_ext(p: &Path) -> PathBuf {
    let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_lowercase();
    for f in ArchiveFormat::ALL {
        let ext = format!(".{}", f.extension());
        if lower.ends_with(&ext) {
            return p.with_file_name(&name[..name.len() - ext.len()]);
        }
    }
    for ext in [".tgz", ".txz", ".tzst", ".tbz2"] {
        if lower.ends_with(ext) {
            return p.with_file_name(&name[..name.len() - ext.len()]);
        }
    }
    p.to_path_buf()
}

fn update_create_controls(dlg: HWND, fmt: ArchiveFormat) {
    let pw = fmt.supports_password();
    for id in [IDC_C_PW, IDC_C_PW2, IDC_C_SHOWPW, IDC_C_PW_LABEL, IDC_C_PW2_LABEL] {
        enable(dlg_item(dlg, id), pw);
    }
    enable(dlg_item(dlg, IDC_C_ENCMETHOD), fmt == ArchiveFormat::Zip);
    enable(dlg_item(dlg, IDC_C_ENCMETHOD_LABEL), fmt == ArchiveFormat::Zip);
    enable(dlg_item(dlg, IDC_C_ENCNAMES), fmt == ArchiveFormat::SevenZ);
    enable(dlg_item(dlg, IDC_C_SOLID), fmt == ArchiveFormat::SevenZ);
    enable(dlg_item(dlg, IDC_C_SPLIT), matches!(fmt, ArchiveFormat::Zip | ArchiveFormat::SevenZ) || fmt.is_tar());
}

unsafe extern "system" fn create_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let st: &mut CState = store(dlg, lp);
            set_text(dlg, tr(S::CTitle));
            for (id, s) in [
                (IDC_C_PATH_LABEL, S::CPath),
                (IDC_C_BROWSE, S::XBrowse),
                (IDC_C_OPTS_GROUP, S::COpts),
                (IDC_C_FMT_LABEL, S::CFormat),
                (IDC_C_LEVEL_LABEL, S::CLevel),
                (IDC_C_SPLIT_LABEL, S::CSplit),
                (IDC_C_SOLID, S::CSolid),
                (IDC_C_ENC_GROUP, S::CEnc),
                (IDC_C_PW_LABEL, S::CPw),
                (IDC_C_PW2_LABEL, S::CPw2),
                (IDC_C_SHOWPW, S::CShowPw),
                (IDC_C_ENCMETHOD_LABEL, S::CEncMethod),
                (IDC_C_ENCNAMES, S::CEncNames),
                (IDCANCEL, S::GCancel),
            ] {
                set_dlg_text(dlg, id, tr(s));
            }
            set_dlg_text(dlg, IDOK, if super::i18n::is_zh() { "开始压缩" } else { "Compress" });
            for f in ArchiveFormat::WRITABLE {
                combo_add(dlg, IDC_C_FORMAT, f.name());
            }
            let fi = ArchiveFormat::WRITABLE.iter().position(|f| *f == st.format).unwrap_or(0);
            combo_set(dlg, IDC_C_FORMAT, fi as i32);
            for (_, s) in LEVELS {
                combo_add(dlg, IDC_C_LEVEL, tr(s));
            }
            combo_set(dlg, IDC_C_LEVEL, 3);
            for (i, s) in SPLITS.iter().enumerate() {
                combo_add(dlg, IDC_C_SPLIT, if i == 0 { tr(S::CSplitNone) } else { s });
            }
            combo_set(dlg, IDC_C_SPLIT, 0);
            combo_add(dlg, IDC_C_ENCMETHOD, "AES-256");
            combo_add(dlg, IDC_C_ENCMETHOD, "ZipCrypto");
            combo_set(dlg, IDC_C_ENCMETHOD, 0);
            set_checked(dlg, IDC_C_SOLID, true);
            set_password_visible(dlg, &[IDC_C_PW, IDC_C_PW2], false);
            set_dlg_text(dlg, IDC_C_PATH, &st.initial.display().to_string());
            let names: Vec<String> = st
                .sources
                .iter()
                .take(4)
                .map(|p| p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
                .collect();
            let mut list = names.join(", ");
            if st.sources.len() > 4 {
                list.push_str(", …");
            }
            set_dlg_text(dlg, IDC_C_INFO, &trf(S::CInfo, &[("n", &st.sources.len().to_string()), ("list", &list)]));
            update_create_controls(dlg, st.format);
            1
        }
        WM_COMMAND => {
            let id = loword(wp) as i32;
            let code = hiword(wp);
            let Some(st) = ctx::<CState>(dlg) else { return 0 };
            match id {
                IDC_C_FORMAT if code == CBN_SELCHANGE => {
                    let i = combo_sel(dlg, IDC_C_FORMAT).max(0) as usize;
                    let fmt = ArchiveFormat::WRITABLE[i.min(ArchiveFormat::WRITABLE.len() - 1)];
                    st.format = fmt;
                    let cur = PathBuf::from(get_dlg_text(dlg, IDC_C_PATH));
                    let base = strip_archive_ext(&cur);
                    let name = base.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let next = base.with_file_name(format!("{name}.{}", fmt.extension()));
                    set_dlg_text(dlg, IDC_C_PATH, &next.display().to_string());
                    update_create_controls(dlg, fmt);
                }
                IDC_C_SHOWPW => set_password_visible(dlg, &[IDC_C_PW, IDC_C_PW2], is_checked(dlg, IDC_C_SHOWPW)),
                IDC_C_BROWSE => {
                    let cur = PathBuf::from(get_dlg_text(dlg, IDC_C_PATH));
                    if let Some(p) = save_file(dlg, tr(S::MSaveAs), &cur) {
                        set_dlg_text(dlg, IDC_C_PATH, &p.display().to_string());
                    }
                }
                IDOK => {
                    let target = PathBuf::from(get_dlg_text(dlg, IDC_C_PATH).trim());
                    if target.as_os_str().is_empty() {
                        error_box(dlg, tr(S::CNoPath));
                        return 1;
                    }
                    let pw = get_dlg_text(dlg, IDC_C_PW);
                    let pw2 = get_dlg_text(dlg, IDC_C_PW2);
                    let fmt = st.format;
                    if !pw.is_empty() && !fmt.supports_password() {
                        error_box(dlg, &trf(S::CNoPwFormat, &[("fmt", fmt.name())]));
                        return 1;
                    }
                    if fmt.supports_password() && pw != pw2 {
                        error_box(dlg, tr(S::CPwMismatch));
                        return 1;
                    }
                    let split_text = get_dlg_text(dlg, IDC_C_SPLIT);
                    let split = if split_text.trim().is_empty() || split_text == tr(S::CSplitNone) {
                        None
                    } else {
                        let cleaned = split_text.split('(').next().unwrap_or("").trim().to_string();
                        match okzip_core::parse_size(&cleaned) {
                            Some(v) => Some(v),
                            None => {
                                error_box(dlg, &trf(S::CBadSplit, &[("s", &split_text)]));
                                return 1;
                            }
                        }
                    };
                    let target_exists = if split.is_some() {
                        target.with_file_name(format!("{}.001", target.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())).exists()
                    } else {
                        target.exists()
                    };
                    if target_exists && !confirm(dlg, &trf(S::CExists, &[("path", &target.display().to_string())])) {
                        return 1;
                    }
                    let li = combo_sel(dlg, IDC_C_LEVEL).clamp(0, LEVELS.len() as i32 - 1) as usize;
                    let options = CreateOptions {
                        format: fmt,
                        level: LEVELS[li].0,
                        password: if pw.is_empty() || !fmt.supports_password() { None } else { Some(pw) },
                        zip_encryption: if combo_sel(dlg, IDC_C_ENCMETHOD) == 1 { ZipEncryption::ZipCrypto } else { ZipEncryption::Aes256 },
                        encrypt_headers: is_checked(dlg, IDC_C_ENCNAMES),
                        solid: is_checked(dlg, IDC_C_SOLID),
                        split_size: split,
                    };
                    st.out = Some(CreateChoice { target, options });
                    unsafe { EndDialog(dlg, IDOK as isize) };
                }
                IDCANCEL => unsafe {
                    EndDialog(dlg, IDCANCEL as isize);
                },
                _ => {}
            }
            1
        }
        _ => 0,
    }
}

pub fn create_dialog(owner: HWND, sources: &[PathBuf], format: ArchiveFormat) -> Option<CreateChoice> {
    let initial = okzip_core::default_archive_path(sources, format).unwrap_or_else(|| PathBuf::from(format!("archive.{}", format.extension())));
    let mut st = CState { sources: sources.to_vec(), initial, format, out: None };
    run_modal(owner, IDD_CREATE, &mut st, Some(create_proc));
    st.out
}

// ---------------------------------------------------------------- settings

unsafe extern "system" fn settings_proc(dlg: HWND, msg: u32, wp: WPARAM, _lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            set_text(dlg, tr(S::STitle));
            for (id, s) in [
                (IDC_S_SHELL_GROUP, S::SShellGroup),
                (IDC_S_SHELL_TEXT, S::SShellText),
                (IDC_S_REG_MENU, S::SReg),
                (IDC_S_UNREG_MENU, S::SUnreg),
                (IDC_S_ASSOC_GROUP, S::SAssocGroup),
                (IDC_S_ASSOC, S::SAssoc),
                (IDC_S_UNASSOC, S::SUnassoc),
                (IDC_S_DEFAPPS, S::SDefApps),
                (IDC_S_MORE, S::SMore),
                (IDOK, S::SClose),
            ] {
                set_dlg_text(dlg, id, tr(s));
            }
            1
        }
        WM_COMMAND => {
            let id = loword(wp) as i32;
            let result = match id {
                IDC_S_REG_MENU => Some(registry::register_context_menu().map(|_| S::SDoneReg)),
                IDC_S_UNREG_MENU => Some(registry::unregister_context_menu().map(|_| S::SDoneUnreg)),
                IDC_S_ASSOC => Some(registry::associate().map(|_| S::SDoneAssoc)),
                IDC_S_UNASSOC => Some(registry::unassociate().map(|_| S::SDoneUnassoc)),
                IDC_S_DEFAPPS => {
                    shell_open(dlg, Path::new("ms-settings:defaultapps"));
                    None
                }
                IDOK | IDCANCEL => {
                    unsafe { EndDialog(dlg, IDOK as isize) };
                    None
                }
                _ => None,
            };
            if let Some(r) = result {
                let text = match r {
                    Ok(s) => tr(s).to_string(),
                    Err(e) => trf(S::SFailed, &[("err", &e)]),
                };
                set_dlg_text(dlg, IDC_S_STATUS, &text);
            }
            1
        }
        _ => 0,
    }
}

pub fn settings_dialog(owner: HWND) {
    unsafe { DialogBoxParamW(hinstance(), make_int_resource(IDD_SETTINGS), owner, Some(settings_proc), 0) };
}
