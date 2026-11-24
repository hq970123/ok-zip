//! Modal dialogs (templates live in res/app.rc): password, extract, create,
//! settings. Captions are filled from the string tables at WM_INITDIALOG.

use super::i18n::{tr, trf, S};
use super::ids::*;
use super::registry;
use super::settings::{self, ExtractTarget, Language, OverwritePolicy};
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
    dialog_box_res(owner, id, proc_, state as *mut T as isize) == IDOK as isize
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
            set_dlg_text(dlg, IDOK, tr(S::SOk));
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
            let cfg = settings::get();
            let sub = cfg.extract_target != ExtractTarget::CurrentFolder;
            set_checked(dlg, IDC_X_SUBFOLDER, sub);
            let shown = if sub { st.base.join(&st.stem) } else { st.base.clone() };
            set_dlg_text(dlg, IDC_X_DEST, &shown.display().to_string());
            let ow = match cfg.overwrite {
                OverwritePolicy::Ask => IDC_X_OW_ASK,
                OverwritePolicy::Overwrite => IDC_X_OW_OVERWRITE,
                OverwritePolicy::Skip => IDC_X_OW_SKIP,
                OverwritePolicy::Rename => IDC_X_OW_RENAME,
            };
            set_checked(dlg, ow, true);
            set_checked(dlg, IDC_X_OPEN_AFTER, cfg.open_folder_after_extract);
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
/// Index into [`LEVELS`] of the closest level <= `level`.
fn level_index(level: u32) -> usize {
    LEVELS.iter().rposition(|(l, _)| *l <= level).unwrap_or(3)
}

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
            combo_set(dlg, IDC_C_LEVEL, level_index(settings::get().level()) as i32);
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

// ---------------------------------------------------------------- settings (property sheet)

struct Sheet {
    s: settings::Settings,
    applied: bool,
    /// Patched (UI typeface) page templates; must outlive the sheet.
    _templates: Vec<Vec<u32>>,
}

fn sheet_of<'a>(dlg: HWND) -> Option<&'a mut Sheet> {
    ctx::<Sheet>(dlg)
}

/// WM_INITDIALOG of a property page: lParam is the PROPSHEETPAGEW whose
/// lParam is our `Sheet`.
fn page_init(dlg: HWND, lp: LPARAM) -> &'static mut Sheet {
    unsafe {
        let psp = &*(lp as *const PROPSHEETPAGEW);
        store::<Sheet>(dlg, psp.lParam)
    }
}

fn page_result(dlg: HWND, v: isize) -> isize {
    unsafe { SetWindowLongPtrW(dlg, DWLP_MSGRESULT as i32, v) };
    1
}

fn is_apply(lp: LPARAM) -> bool {
    unsafe { (*(lp as *const NMHDR)).code == PSN_APPLY }
}

fn changed(dlg: HWND) {
    unsafe { SendMessageW(GetParent(dlg), PSM_CHANGED, dlg as usize, 0) };
}

fn recent_text(n: usize) -> String {
    trf(S::SRecentCount, &[("n", &n.to_string())])
}

unsafe extern "system" fn page_general(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let sh = page_init(dlg, lp);
            for (id, s) in [
                (IDC_S_UI_GROUP, S::SUiGroup),
                (IDC_S_LANG_LABEL, S::SLang),
                (IDC_S_LANG_NOTE, S::SLangNote),
                (IDC_S_WIN_GROUP, S::SWinGroup),
                (IDC_S_REMEMBER, S::SRemember),
                (IDC_S_RECENT_GROUP, S::SRecentGroup),
                (IDC_S_RECENT_CLEAR, S::SRecentClear),
            ] {
                set_dlg_text(dlg, id, tr(s));
            }
            for s in [S::SLangAuto, S::SLangZh, S::SLangEn] {
                combo_add(dlg, IDC_S_LANG, tr(s));
            }
            combo_set(dlg, IDC_S_LANG, match sh.s.language {
                Language::Auto => 0,
                Language::Zh => 1,
                Language::En => 2,
            });
            set_checked(dlg, IDC_S_REMEMBER, sh.s.remember_window);
            set_dlg_text(dlg, IDC_S_RECENT_TEXT, &recent_text(sh.s.recent.len()));
            enable(dlg_item(dlg, IDC_S_RECENT_CLEAR), !sh.s.recent.is_empty());
            set_dlg_text(dlg, IDC_S_FILE, &trf(S::SFile, &[("path", &settings::settings_path().display().to_string())]));
            1
        }
        WM_COMMAND => {
            let id = loword(wp) as i32;
            if id == IDC_S_RECENT_CLEAR {
                if let Some(sh) = sheet_of(dlg) {
                    sh.s.recent.clear();
                }
                set_dlg_text(dlg, IDC_S_RECENT_TEXT, &recent_text(0));
                enable(dlg_item(dlg, IDC_S_RECENT_CLEAR), false);
                changed(dlg);
            } else if id == IDC_S_REMEMBER || (id == IDC_S_LANG && hiword(wp) == CBN_SELCHANGE) {
                changed(dlg);
            }
            0
        }
        WM_NOTIFY if is_apply(lp) => {
            if let Some(sh) = sheet_of(dlg) {
                sh.s.language = match combo_sel(dlg, IDC_S_LANG) {
                    1 => Language::Zh,
                    2 => Language::En,
                    _ => Language::Auto,
                };
                sh.s.remember_window = is_checked(dlg, IDC_S_REMEMBER);
                sh.applied = true;
            }
            page_result(dlg, PSNRET_NOERROR as isize)
        }
        _ => 0,
    }
}

const XT_IDS: [(ExtractTarget, i32); 3] =
    [(ExtractTarget::CurrentFolder, IDC_S_XT_HERE), (ExtractTarget::SubfolderSmart, IDC_S_XT_SMART), (ExtractTarget::Ask, IDC_S_XT_ASK)];
const OW_IDS: [(OverwritePolicy, i32); 4] = [
    (OverwritePolicy::Ask, IDC_S_OW_ASK),
    (OverwritePolicy::Overwrite, IDC_S_OW_OVERWRITE),
    (OverwritePolicy::Skip, IDC_S_OW_SKIP),
    (OverwritePolicy::Rename, IDC_S_OW_RENAME),
];

unsafe extern "system" fn page_extract(dlg: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let sh = page_init(dlg, lp);
            for (id, s) in [
                (IDC_S_XT_GROUP, S::SXTargetGroup),
                (IDC_S_XT_HERE, S::SXTargetHere),
                (IDC_S_XT_SMART, S::SXTargetSmart),
                (IDC_S_XT_ASK, S::SXTargetAsk),
                (IDC_S_OW_GROUP, S::XOwGroup),
                (IDC_S_OW_ASK, S::XOwAsk),
                (IDC_S_OW_OVERWRITE, S::XOwOverwrite),
                (IDC_S_OW_SKIP, S::XOwSkip),
                (IDC_S_OW_RENAME, S::XOwRename),
                (IDC_S_OPEN_AFTER, S::SXOpenAfter),
            ] {
                set_dlg_text(dlg, id, tr(s));
            }
            for (v, id) in XT_IDS {
                set_checked(dlg, id, v == sh.s.extract_target);
            }
            for (v, id) in OW_IDS {
                set_checked(dlg, id, v == sh.s.overwrite);
            }
            set_checked(dlg, IDC_S_OPEN_AFTER, sh.s.open_folder_after_extract);
            1
        }
        WM_COMMAND => {
            changed(dlg);
            0
        }
        WM_NOTIFY if is_apply(lp) => {
            if let Some(sh) = sheet_of(dlg) {
                if let Some((v, _)) = XT_IDS.iter().find(|(_, id)| is_checked(dlg, *id)) {
                    sh.s.extract_target = *v;
                }
                if let Some((v, _)) = OW_IDS.iter().find(|(_, id)| is_checked(dlg, *id)) {
                    sh.s.overwrite = *v;
                }
                sh.s.open_folder_after_extract = is_checked(dlg, IDC_S_OPEN_AFTER);
                sh.applied = true;
            }
            page_result(dlg, PSNRET_NOERROR as isize)
        }
        _ => 0,
    }
}

unsafe extern "system" fn page_compress(dlg: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let sh = page_init(dlg, lp);
            for (id, s) in [(IDC_S_C_GROUP, S::SCGroup), (IDC_S_C_FMT_LABEL, S::CFormat), (IDC_S_C_LEVEL_LABEL, S::CLevel), (IDC_S_C_NOTE, S::SCNote)] {
                set_dlg_text(dlg, id, tr(s));
            }
            for f in ArchiveFormat::WRITABLE {
                combo_add(dlg, IDC_S_C_FMT, f.name());
            }
            let fi = ArchiveFormat::WRITABLE.iter().position(|f| *f == sh.s.format()).unwrap_or(0);
            combo_set(dlg, IDC_S_C_FMT, fi as i32);
            for (_, s) in LEVELS {
                combo_add(dlg, IDC_S_C_LEVEL, tr(s));
            }
            combo_set(dlg, IDC_S_C_LEVEL, level_index(sh.s.level()) as i32);
            1
        }
        WM_COMMAND => {
            if hiword(_wp) == CBN_SELCHANGE {
                changed(dlg);
            }
            0
        }
        WM_NOTIFY if is_apply(lp) => {
            if let Some(sh) = sheet_of(dlg) {
                let fi = combo_sel(dlg, IDC_S_C_FMT).clamp(0, ArchiveFormat::WRITABLE.len() as i32 - 1) as usize;
                sh.s.set_format(ArchiveFormat::WRITABLE[fi]);
                let li = combo_sel(dlg, IDC_S_C_LEVEL).clamp(0, LEVELS.len() as i32 - 1) as usize;
                sh.s.create_level = LEVELS[li].0;
                sh.applied = true;
            }
            page_result(dlg, PSNRET_NOERROR as isize)
        }
        _ => 0,
    }
}

fn shell_state(dlg: HWND) {
    let s = if !registry::context_menu_registered() {
        S::SShellOff
    } else if registry::serving_package().is_some() {
        S::SShellOnPkg
    } else {
        S::SShellOn
    };
    set_dlg_text(dlg, IDC_S_STATE, tr(s));
}

unsafe extern "system" fn page_shell(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            page_init(dlg, lp);
            for (id, s) in [
                (IDC_S_SHELL_GROUP, S::SShellGroup),
                (IDC_S_SHELL_TEXT, S::SShellText),
                (IDC_S_REG_MENU, S::SReg),
                (IDC_S_UNREG_MENU, S::SUnreg),
                (IDC_S_ASSOC_GROUP, S::SAssocGroup),
                (IDC_S_ASSOC, S::SAssoc),
                (IDC_S_UNASSOC, S::SUnassoc),
                (IDC_S_DEFAPPS, S::SDefApps),
            ] {
                set_dlg_text(dlg, id, tr(s));
            }
            shell_state(dlg);
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
                _ => None,
            };
            if let Some(r) = result {
                let text = match r {
                    Ok(s) => tr(s).to_string(),
                    Err(e) => trf(S::SFailed, &[("err", &e)]),
                };
                set_dlg_text(dlg, IDC_S_STATUS, &text);
                shell_state(dlg);
            }
            0
        }
        _ => 0,
    }
}

/// Give the sheet frame (tab strip, OK/Cancel) the UI typeface too.
unsafe extern "system" fn sheet_callback(hwnd: HWND, msg: u32, _lp: LPARAM) -> i32 {
    if msg == PSCB_INITIALIZED {
        let font = create_ui_font(dpi_of(hwnd)); // lives as long as the process; one per sheet
        unsafe extern "system" fn apply(child: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
            unsafe {
                // Skip the pages (they already use the patched template font).
                let mut cls = [0u16; 16];
                let n = GetClassNameW(child, cls.as_mut_ptr(), 16);
                let is_dialog = from_wide(&cls[..n.max(0) as usize]) == "#32770";
                if !is_dialog && GetParent(child) == GetAncestor(child, GA_ROOT) {
                    SendMessageW(child, WM_SETFONT, lp as usize, 1);
                }
            }
            1
        }
        unsafe { EnumChildWindows(hwnd, Some(apply), font as LPARAM) };
        // comctl32 labels these in the *Windows* language; use ours.
        set_dlg_text(hwnd, IDOK, tr(S::SOk));
        set_dlg_text(hwnd, IDCANCEL, tr(S::GCancel));
    }
    0
}

/// The tabbed settings sheet (常规 / 解压 / 压缩 / 右键菜单与关联). Saves on OK.
/// `start_page` selects the initial tab.
pub fn settings_dialog(owner: HWND, start_page: u32) {
    let before = settings::get();
    let pages: [(i32, S, DLGPROC); 4] = [
        (IDD_PAGE_GENERAL, S::STabGeneral, Some(page_general)),
        (IDD_PAGE_EXTRACT, S::STabExtract, Some(page_extract)),
        (IDD_PAGE_COMPRESS, S::STabCompress, Some(page_compress)),
        (IDD_PAGE_SHELL, S::STabShell, Some(page_shell)),
    ];
    let templates: Vec<Option<Vec<u32>>> = pages.iter().map(|(id, _, _)| dialog_template(*id)).collect();
    let titles: Vec<Vec<u16>> = pages.iter().map(|(_, s, _)| wide(tr(*s))).collect();
    let mut sheet = Box::new(Sheet { s: before.clone(), applied: false, _templates: vec![] });
    let sheet_ptr = &mut *sheet as *mut Sheet as isize;
    let mut psp: Vec<PROPSHEETPAGEW> = Vec::new();
    for (i, (id, _, proc_)) in pages.iter().enumerate() {
        let mut p: PROPSHEETPAGEW = unsafe { std::mem::zeroed() };
        p.dwSize = std::mem::size_of::<PROPSHEETPAGEW>() as u32;
        p.hInstance = hinstance();
        p.dwFlags = PSP_USETITLE;
        match &templates[i] {
            Some(t) => {
                p.dwFlags |= PSP_DLGINDIRECT;
                p.Anonymous1.pResource = t.as_ptr() as *mut DLGTEMPLATE;
            }
            None => p.Anonymous1.pszTemplate = make_int_resource(*id),
        }
        p.pszTitle = titles[i].as_ptr();
        p.pfnDlgProc = *proc_;
        p.lParam = sheet_ptr;
        psp.push(p);
    }
    let caption = wide(tr(S::STitle));
    let mut h: PROPSHEETHEADERW_V2 = unsafe { std::mem::zeroed() };
    h.dwSize = std::mem::size_of::<PROPSHEETHEADERW_V2>() as u32;
    h.dwFlags = PSH_PROPSHEETPAGE | PSH_NOAPPLYNOW | PSH_NOCONTEXTHELP | PSH_USECALLBACK;
    h.hwndParent = owner;
    h.hInstance = hinstance();
    h.pszCaption = caption.as_ptr();
    h.nPages = psp.len() as u32;
    h.Anonymous2.nStartPage = start_page.min(psp.len() as u32 - 1);
    h.Anonymous3.ppsp = psp.as_mut_ptr();
    h.pfnCallback = Some(sheet_callback);
    unsafe { PropertySheetW(&mut h) };
    sheet._templates = templates.into_iter().flatten().collect();
    if sheet.applied {
        let new = sheet.s.clone();
        settings::update(|s| {
            // Window state may have changed while the sheet was open; keep it.
            let (w, c, sp, t) = (s.window, s.column_widths.clone(), s.splitter, s.show_tree);
            *s = new.clone();
            s.window = w;
            s.column_widths = c;
            s.splitter = sp;
            s.show_tree = t;
        });
        if new.language != before.language {
            info_box(owner, tr(S::SLangChanged));
        }
    }
}

// ---------------------------------------------------------------- overwrite conflict

/// Everything the conflict dialog shows, plus the user's answer.
pub struct ConflictQuestion {
    pub target: PathBuf,
    pub old_size: Option<u64>,
    pub old_modified: Option<i64>,
    pub new_size: Option<u64>,
    pub new_modified: Option<i64>,
    pub answer: okzip_core::ConflictChoice,
}

struct KState<'a> {
    q: &'a mut ConflictQuestion,
    bold: windows_sys::Win32::Graphics::Gdi::HFONT,
}

fn describe(size: Option<u64>, time: Option<i64>, newer: bool) -> String {
    let size = size
        .map(|n| if n < 1024 { format!("{n} B") } else { format!("{} ({} B)", okzip_core::format_size(n), group_digits(n)) })
        .unwrap_or_else(|| tr(S::KUnknown).into());
    let mut time = if time.is_some() { format_time_ex(time, true) } else { tr(S::KUnknown).into() };
    if newer {
        time.push_str(tr(S::KNewer));
    }
    trf(S::KInfo, &[("size", &size), ("time", &time)])
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

unsafe extern "system" fn conflict_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, GetObjectW, LOGFONTW};
    use okzip_core::ConflictChoice as C;
    match msg {
        WM_INITDIALOG => {
            unsafe { SetWindowLongPtrW(dlg, GWLP_USERDATA, lp) };
            let st = unsafe { &mut *(lp as *mut KState) };
            set_text(dlg, tr(S::KTitle));
            for (id, s) in [
                (IDC_K_HEADING, S::KHeading),
                (IDC_K_OLD_GROUP, S::KExisting),
                (IDC_K_NEW_GROUP, S::KNew),
                (IDC_K_ALL, S::KApplyAll),
                (IDC_K_OVERWRITE, S::KOverwrite),
                (IDC_K_SKIP, S::KSkip),
                (IDC_K_RENAME, S::KRename),
                (IDCANCEL, S::GCancel),
            ] {
                set_dlg_text(dlg, id, tr(s));
            }
            let q = &*st.q;
            let name = q.target.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            set_dlg_text(dlg, IDC_K_NAME, &name);
            set_dlg_text(dlg, IDC_K_PATH, &q.target.parent().map(|p| p.display().to_string()).unwrap_or_default());
            let new_newer = matches!((q.new_modified, q.old_modified), (Some(n), Some(o)) if n > o);
            let old_newer = matches!((q.new_modified, q.old_modified), (Some(n), Some(o)) if o > n);
            set_dlg_text(dlg, IDC_K_OLD, &describe(q.old_size, q.old_modified, old_newer));
            set_dlg_text(dlg, IDC_K_NEW, &describe(q.new_size, q.new_modified, new_newer));
            unsafe {
                let icon = LoadIconW(std::ptr::null_mut(), IDI_WARNING);
                SendDlgItemMessageW(dlg, IDC_K_ICON, STM_SETICON, icon as usize, 0);
                // bold file name
                let f = SendMessageW(dlg, WM_GETFONT, 0, 0);
                let mut lf: LOGFONTW = std::mem::zeroed();
                if f != 0 && GetObjectW(f as _, std::mem::size_of::<LOGFONTW>() as i32, &mut lf as *mut _ as *mut _) != 0 {
                    lf.lfWeight = 700;
                    st.bold = CreateFontIndirectW(&lf);
                    set_font(dlg_item(dlg, IDC_K_NAME), st.bold);
                }
                windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(dlg_item(dlg, IDC_K_OVERWRITE));
            }
            0
        }
        WM_COMMAND => {
            let id = loword(wp) as i32;
            let all = is_checked(dlg, IDC_K_ALL);
            let choice = match id {
                IDC_K_OVERWRITE => Some(if all { C::OverwriteAll } else { C::Overwrite }),
                IDC_K_SKIP => Some(if all { C::SkipAll } else { C::Skip }),
                IDC_K_RENAME => Some(if all { C::RenameAll } else { C::Rename }),
                IDCANCEL => Some(C::Cancel),
                _ => None,
            };
            if let Some(c) = choice {
                if let Some(st) = ctx::<KState>(dlg) {
                    st.q.answer = c;
                }
                unsafe { EndDialog(dlg, id as isize) };
            }
            1
        }
        WM_DESTROY => {
            if let Some(st) = ctx::<KState>(dlg) {
                if !st.bold.is_null() {
                    unsafe { windows_sys::Win32::Graphics::Gdi::DeleteObject(st.bold as _) };
                }
            }
            0
        }
        _ => 0,
    }
}

/// Ask the user what to do about an existing file (must run on the UI thread).
pub fn conflict_dialog(owner: HWND, q: &mut ConflictQuestion) {
    q.answer = okzip_core::ConflictChoice::Cancel;
    let mut st = KState { q, bold: std::ptr::null_mut() };
    dialog_box_res(owner, IDD_CONFLICT, Some(conflict_proc), &mut st as *mut KState as isize);
}
