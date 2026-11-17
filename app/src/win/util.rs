//! Small Win32 helpers: wide strings, message boxes, DPI, fonts, shell dialogs.

use super::i18n::{tr, S};
use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// NUL-terminated UTF-16.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer (up to the first NUL) → String.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn path_from_wide(buf: &[u16]) -> PathBuf {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    PathBuf::from(OsString::from_wide(&buf[..end]))
}

pub fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(null()) }
}

/// `MAKEINTRESOURCEW`.
pub fn make_int_resource(id: i32) -> *const u16 {
    id as u16 as usize as *const u16
}

pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}
pub fn hiword(v: usize) -> u32 {
    ((v >> 16) & 0xFFFF) as u32
}

pub fn message_box(owner: HWND, text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = wide(text);
    let c = wide(tr(S::AppName));
    unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), style) }
}

pub fn error_box(owner: HWND, text: &str) {
    message_box(owner, text, MB_OK | MB_ICONERROR);
}

pub fn info_box(owner: HWND, text: &str) {
    message_box(owner, text, MB_OK | MB_ICONINFORMATION);
}

pub fn confirm(owner: HWND, text: &str) -> bool {
    message_box(owner, text, MB_YESNO | MB_ICONQUESTION) == IDYES
}

pub fn set_text(hwnd: HWND, text: &str) {
    let w = wide(text);
    unsafe { SetWindowTextW(hwnd, w.as_ptr()) };
}

pub fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; len as usize + 1];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        from_wide(&buf)
    }
}

pub fn dlg_item(dlg: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(dlg, id) }
}

pub fn set_dlg_text(dlg: HWND, id: i32, text: &str) {
    set_text(dlg_item(dlg, id), text);
}

pub fn get_dlg_text(dlg: HWND, id: i32) -> String {
    get_text(dlg_item(dlg, id))
}

pub fn is_checked(dlg: HWND, id: i32) -> bool {
    unsafe { windows_sys::Win32::UI::Controls::IsDlgButtonChecked(dlg, id) == 1 }
}

pub fn set_checked(dlg: HWND, id: i32, on: bool) {
    unsafe { windows_sys::Win32::UI::Controls::CheckDlgButton(dlg, id, if on { 1 } else { 0 }) };
}

pub fn enable(hwnd: HWND, on: bool) {
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(hwnd, on as i32) };
}

pub fn dpi_of(hwnd: HWND) -> u32 {
    let d = unsafe { GetDpiForWindow(hwnd) };
    if d == 0 {
        96
    } else {
        d
    }
}

pub fn scale(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}

/// UI typeface: Microsoft YaHei UI for the Chinese UI (so Simplified Chinese
/// never falls back glyph-by-glyph to a Japanese font on non-Chinese Windows),
/// Segoe UI otherwise.
pub fn ui_face() -> &'static str {
    if super::i18n::is_zh() {
        "Microsoft YaHei UI"
    } else {
        "Segoe UI"
    }
}

/// The UI font (system message font metrics, [`ui_face`] typeface) at the given DPI.
pub fn create_ui_font(dpi: u32) -> HFONT {
    unsafe {
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        let ok = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            &mut ncm as *mut _ as *mut _,
            0,
            dpi,
        );
        let mut lf = if ok != 0 { ncm.lfMessageFont } else { std::mem::zeroed() };
        if ok == 0 {
            lf.lfHeight = -scale(12, dpi);
            lf.lfWeight = 400;
        }
        let face = wide(ui_face());
        lf.lfFaceName = [0; 32];
        lf.lfFaceName[..face.len()].copy_from_slice(&face);
        CreateFontIndirectW(&lf)
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe { SendMessageW(hwnd, WM_SETFONT, font as usize, 1) };
}

/// Folder picker (SHBrowseForFolder, new-style dialog).
pub fn browse_folder(owner: HWND, title: &str, initial: &Path) -> Option<PathBuf> {
    unsafe extern "system" fn cb(hwnd: HWND, msg: u32, _l: LPARAM, data: LPARAM) -> i32 {
        if msg == BFFM_INITIALIZED && data != 0 {
            SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, data);
        }
        0
    }
    unsafe {
        let t = wide(title);
        let init = wide_path(initial);
        let mut name = [0u16; MAX_PATH as usize];
        let mut bi: BROWSEINFOW = std::mem::zeroed();
        bi.hwndOwner = owner;
        bi.pszDisplayName = name.as_mut_ptr();
        bi.lpszTitle = t.as_ptr();
        bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_EDITBOX;
        bi.lpfn = Some(cb);
        bi.lParam = init.as_ptr() as isize;
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let ok = SHGetPathFromIDListEx(pidl, buf.as_mut_ptr(), buf.len() as u32, 0);
        CoTaskMemFree(pidl as *const _);
        if ok != 0 {
            Some(path_from_wide(&buf))
        } else {
            None
        }
    }
}

/// Build a "Name\0*.a;*.b\0...\0\0" filter string.
fn filter(pairs: &[(&str, &str)]) -> Vec<u16> {
    let mut v = Vec::new();
    for (name, pat) in pairs {
        v.extend(name.encode_utf16());
        v.push(0);
        v.extend(pat.encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

pub fn archive_filter_pattern() -> String {
    okzip_core::ASSOCIATED_EXTENSIONS
        .iter()
        .map(|e| format!("*.{e}"))
        .chain(["*.tar.gz", "*.tar.bz2", "*.tar.xz", "*.tar.zst", "*.jar"].iter().map(|s| s.to_string()))
        .collect::<Vec<_>>()
        .join(";")
}

/// Standard Open dialog. `multi` allows several files.
pub fn open_files(owner: HWND, title: &str, archives_only: bool, multi: bool) -> Vec<PathBuf> {
    unsafe {
        let pat = archive_filter_pattern();
        let all = (tr(S::MAllFiles), "*.*");
        let filt = if archives_only { filter(&[(tr(S::MOpenFilter), &pat), all]) } else { filter(&[all]) };
        let t = wide(title);
        let mut buf = vec![0u16; 65536];
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filt.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = t.as_ptr();
        ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
        if multi {
            ofn.Flags |= OFN_ALLOWMULTISELECT;
        }
        if GetOpenFileNameW(&mut ofn) == 0 {
            return vec![];
        }
        // Multi-select: dir\0file1\0file2\0\0 ; single: full path\0
        let mut parts: Vec<PathBuf> = Vec::new();
        let mut start = 0;
        for i in 0..buf.len() {
            if buf[i] == 0 {
                if i == start {
                    break;
                }
                parts.push(PathBuf::from(OsString::from_wide(&buf[start..i])));
                start = i + 1;
            }
        }
        if parts.len() <= 1 {
            parts
        } else {
            let dir = parts.remove(0);
            parts.into_iter().map(|f| dir.join(f)).collect()
        }
    }
}

/// Standard Save As dialog.
pub fn save_file(owner: HWND, title: &str, initial: &Path) -> Option<PathBuf> {
    unsafe {
        let pat = archive_filter_pattern();
        let filt = filter(&[(tr(S::MOpenFilter), &pat), (tr(S::MAllFiles), "*.*")]);
        let t = wide(title);
        let mut buf = vec![0u16; 32768];
        let init = wide_path(initial);
        let n = init.len().min(buf.len() - 1);
        buf[..n].copy_from_slice(&init[..n]);
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filt.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = t.as_ptr();
        ofn.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
        if GetSaveFileNameW(&mut ofn) == 0 {
            None
        } else {
            Some(path_from_wide(&buf))
        }
    }
}

/// ShellExecute "open" on a file or folder.
pub fn shell_open(owner: HWND, target: &Path) {
    let p = wide_path(target);
    let verb = wide("open");
    unsafe { ShellExecuteW(owner, verb.as_ptr(), p.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

/// Open Explorer with `target` selected.
pub fn reveal_in_explorer(target: &Path) {
    let args = wide(&format!("/select,\"{}\"", target.display()));
    let exe = wide("explorer.exe");
    let verb = wide("open");
    unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), exe.as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL) };
}

/// System small-icon image list (shared, must not be destroyed).
pub fn system_image_list() -> isize {
    unsafe {
        let mut sfi: SHFILEINFOW = std::mem::zeroed();
        let dummy = wide("C:\\");
        SHGetFileInfoW(
            dummy.as_ptr(),
            0,
            &mut sfi,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_SMALLICON,
        ) as isize
    }
}

#[derive(Clone)]
pub struct IconInfo {
    pub icon: i32,
    pub type_name: String,
}

static ICONS: Mutex<Option<HashMap<String, IconInfo>>> = Mutex::new(None);

/// Icon index + type name for a file name (by extension) or folder, cached.
pub fn icon_for(name: &str, is_dir: bool, open: bool) -> IconInfo {
    let key = if is_dir {
        format!("<dir:{open}>")
    } else {
        match name.rfind('.') {
            Some(i) if i > 0 => name[i..].to_lowercase(),
            _ => "<noext>".to_string(),
        }
    };
    if let Some(hit) = ICONS.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&key).cloned())) {
        return hit;
    }
    let info = unsafe {
        let mut sfi: SHFILEINFOW = std::mem::zeroed();
        let probe = if is_dir { wide("folder") } else { wide(&format!("file{}", if key.starts_with('<') { "" } else { &key })) };
        let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
        let mut flags = SHGFI_USEFILEATTRIBUTES | SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_TYPENAME;
        if open {
            flags |= SHGFI_OPENICON;
        }
        SHGetFileInfoW(probe.as_ptr(), attrs, &mut sfi, std::mem::size_of::<SHFILEINFOW>() as u32, flags);
        IconInfo { icon: sfi.iIcon, type_name: from_wide(&sfi.szTypeName) }
    };
    if let Ok(mut g) = ICONS.lock() {
        g.get_or_insert_with(HashMap::new).insert(key, info.clone());
    }
    info
}

/// Icon index of a real file on disk (used for the tree root).
pub fn icon_for_path(p: &Path) -> i32 {
    unsafe {
        let mut sfi: SHFILEINFOW = std::mem::zeroed();
        let w = wide_path(p);
        SHGetFileInfoW(w.as_ptr(), 0, &mut sfi, std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
        sfi.iIcon
    }
}

/// Unix seconds → local "YYYY-MM-DD HH:MM".
pub fn format_time(unix: Option<i64>) -> String {
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let Some(t) = unix else { return String::new() };
    let ticks = (t + 11_644_473_600) as u64 * 10_000_000;
    unsafe {
        let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut utc) == 0 || SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local) == 0 {
            return String::new();
        }
        format!("{:04}-{:02}-{:02} {:02}:{:02}", local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
    }
}

/// "RHSDA"-style attribute letters.
pub fn attr_string(a: u32) -> String {
    let mut s = String::new();
    for (bit, ch) in [(0x01, 'R'), (0x02, 'H'), (0x04, 'S'), (0x10, 'D'), (0x20, 'A')] {
        if a & bit != 0 {
            s.push(ch);
        }
    }
    s
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("okzip.exe"))
}

/// Seconds → "mm:ss" / "h:mm:ss".
pub fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// HWND that can cross threads (only used to post messages / own message boxes).
#[derive(Clone, Copy)]
pub struct SendHwnd(pub usize);
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}
impl SendHwnd {
    pub fn new(h: HWND) -> Self {
        SendHwnd(h as usize)
    }
    pub fn get(self) -> HWND {
        self.0 as HWND
    }
}

/// Load a DIALOGEX template from the resources with its FONT typeface
/// replaced by [`ui_face`], for DialogBoxIndirectParamW / CreateDialogIndirectParamW.
/// Returned as u32s so the buffer is DWORD-aligned. `None` if it can't be parsed.
pub fn dialog_template(id: i32) -> Option<Vec<u32>> {
    use windows_sys::Win32::System::LibraryLoader::{FindResourceW, LoadResource, LockResource, SizeofResource};
    unsafe {
        let hinst = hinstance();
        let res = FindResourceW(hinst, make_int_resource(id), make_int_resource(5) /* RT_DIALOG */);
        if res.is_null() {
            return None;
        }
        let size = SizeofResource(hinst, res) as usize;
        let data = LockResource(LoadResource(hinst, res)) as *const u16;
        if data.is_null() || size < 26 {
            return None;
        }
        let w = std::slice::from_raw_parts(data, size / 2);
        if w[0] != 1 || w[1] != 0xFFFF {
            return None; // not DIALOGEX
        }
        let style = (w[6] as u32) | ((w[7] as u32) << 16);
        if style & DS_SETFONT as u32 == 0 {
            return None;
        }
        let mut i = 13usize;
        let skip_sz_or_ord = |i: &mut usize| -> Option<()> {
            match *w.get(*i)? {
                0 => *i += 1,
                0xFFFF => *i += 2,
                _ => {
                    while *w.get(*i)? != 0 {
                        *i += 1;
                    }
                    *i += 1;
                }
            }
            Some(())
        };
        skip_sz_or_ord(&mut i)?; // menu
        skip_sz_or_ord(&mut i)?; // class
        while *w.get(i)? != 0 {
            i += 1; // title
        }
        i += 1;
        i += 3; // pointsize, weight, italic+charset
        let face_start = i;
        while *w.get(i)? != 0 {
            i += 1;
        }
        i += 1;
        let items = (i + 1) & !1; // first item is DWORD-aligned
        let mut out: Vec<u16> = w[..face_start].to_vec();
        out.extend(ui_face().encode_utf16());
        out.push(0);
        if out.len() % 2 == 1 {
            out.push(0);
        }
        out.extend_from_slice(w.get(items..)?);
        if out.len() % 2 == 1 {
            out.push(0);
        }
        Some(out.chunks(2).map(|c| c[0] as u32 | ((c[1] as u32) << 16)).collect())
    }
}

/// Modal dialog from a resource template, using the UI typeface.
pub fn dialog_box_res(owner: HWND, id: i32, proc_: DLGPROC, param: isize) -> isize {
    unsafe {
        match dialog_template(id) {
            Some(t) => DialogBoxIndirectParamW(hinstance(), t.as_ptr() as *const DLGTEMPLATE, owner, proc_, param),
            None => DialogBoxParamW(hinstance(), make_int_resource(id), owner, proc_, param),
        }
    }
}

/// Modeless dialog from a resource template, using the UI typeface.
pub fn create_dialog_res(owner: HWND, id: i32, proc_: DLGPROC, param: isize) -> HWND {
    unsafe {
        match dialog_template(id) {
            Some(t) => CreateDialogIndirectParamW(hinstance(), t.as_ptr() as *const DLGTEMPLATE, owner, proc_, param),
            None => CreateDialogParamW(hinstance(), make_int_resource(id), owner, proc_, param),
        }
    }
}
