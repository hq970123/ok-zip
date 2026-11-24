//! Explorer integration via per-user registry keys (HKCU\Software\Classes):
//! a cascading "OK Zip" context submenu (ExtendedSubCommandsKey) and "Open with" file associations. No admin
//! rights required.

use super::i18n::{tr, S};
use super::util::{exe_path, wide};
use okzip_core::ASSOCIATED_EXTENSIONS;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

const CLASSES: &str = r"Software\Classes";
const PROGID: &str = "OKZip.Archive";
const VERB_PREFIX: &str = "OKZip.";

type R = Result<(), String>;

fn set_value(path: &str, name: Option<&str>, value: &str) -> R {
    unsafe {
        let p = wide(&format!(r"{CLASSES}\{path}"));
        let mut key: HKEY = null_mut();
        let rc = RegCreateKeyExW(HKEY_CURRENT_USER, p.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, null(), &mut key, null_mut());
        if rc != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyEx({path}) = {rc}"));
        }
        let n = name.map(wide);
        let v = wide(value);
        let rc = RegSetValueExW(
            key,
            n.as_ref().map_or(null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            v.as_ptr() as *const u8,
            (v.len() * 2) as u32,
        );
        RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return Err(format!("RegSetValueEx({path}) = {rc}"));
        }
    }
    Ok(())
}

fn delete_tree(path: &str) {
    unsafe {
        let p = wide(&format!(r"{CLASSES}\{path}"));
        RegDeleteTreeW(HKEY_CURRENT_USER, p.as_ptr());
        RegDeleteKeyW(HKEY_CURRENT_USER, p.as_ptr());
    }
}

fn delete_value(path: &str, name: &str) {
    unsafe {
        let p = wide(&format!(r"{CLASSES}\{path}"));
        let n = wide(name);
        RegDeleteKeyValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr());
    }
}

fn notify() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
}

fn exe() -> String {
    exe_path().display().to_string()
}

/// Old (v0.1) flat verb names, removed on (un)registration.
const LEGACY_ARCHIVE_VERBS: [&str; 3] = ["Open", "ExtractHere", "ExtractTo"];
const LEGACY_COMPRESS_VERBS: [&str; 2] = ["CompressZip", "Compress7z"];

/// Name of the cascading parent verb under `...\shell\`.
const MENU_VERB: &str = "OKZip";
/// Sub-command containers (`HKCU\Software\Classes\<key>\shell\NN<verb>`),
/// referenced through `ExtendedSubCommandsKey` so no shell-extension DLL and
/// no HKLM CommandStore entry are needed.
const ARCHIVE_MENU: &str = "OKZip.ArchiveMenu";
const COMPRESS_MENU: &str = "OKZip.CompressMenu";

fn key_exists(path: &str) -> bool {
    unsafe {
        let p = wide(&format!(r"{CLASSES}\{path}"));
        let mut key: HKEY = null_mut();
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, p.as_ptr(), 0, KEY_READ, &mut key);
        if rc == ERROR_SUCCESS {
            RegCloseKey(key);
            true
        } else {
            false
        }
    }
}

fn remove_legacy_verbs() {
    for ext in ASSOCIATED_EXTENSIONS {
        for verb in LEGACY_ARCHIVE_VERBS {
            delete_tree(&format!(r"SystemFileAssociations\.{ext}\shell\{VERB_PREFIX}{verb}"));
        }
    }
    for base in ["*", "Directory"] {
        for verb in LEGACY_COMPRESS_VERBS {
            delete_tree(&format!(r"{base}\shell\{VERB_PREFIX}{verb}"));
        }
    }
}

/// The menu as `okzip-shellmenu` describes it, with labels in the UI
/// language and icons from okzip.exe.
pub fn shell_menu() -> okzip_shellmenu::context_menu::Menu {
    let exe = exe_path();
    let labels = okzip_shellmenu::Labels {
        title: tr(S::VMenu).into(),
        open: tr(S::VOpen).into(),
        extract_here: tr(S::VExtractHere).into(),
        extract_smart: tr(S::VExtractTo).into(),
        test: tr(S::VTest).into(),
        compress_zip: tr(S::VCompressZip).into(),
        compress_7z: tr(S::VCompress7z).into(),
    };
    okzip_shellmenu::menu(&exe, &labels, &okzip_shellmenu::Icons::windows_exe(&exe), ASSOCIATED_EXTENSIONS)
}

/// Remove the v0.1/v0.2 hand-written verbs (now written by context-menu-rs).
fn remove_own_verbs() {
    remove_legacy_verbs();
    for ext in ASSOCIATED_EXTENSIONS {
        delete_tree(&format!(r"SystemFileAssociations\.{ext}\shell\{MENU_VERB}"));
    }
    for base in ["*", "Directory"] {
        delete_tree(&format!(r"{base}\shell\{MENU_VERB}"));
    }
    delete_tree(ARCHIVE_MENU);
    delete_tree(COMPRESS_MENU);
}

/// One "OK Zip" submenu in the Explorer context menu (per user), through
/// context-menu-rs: archives get 打开 / 解压到当前文件夹 / 解压到同名文件夹 / 测试 /
/// 压缩为 ZIP / 压缩为 7Z, other files and folders the two compress commands.
/// Without OK Zip's sparse package: classic verbs (Windows 10, Windows 11
/// "显示更多选项"). With it registered (installer, Windows 11): no verbs; the
/// package's IExplorerCommand DLL serves the top-level menu and the classic
/// one from the definition written here.
pub fn register_context_menu() -> R {
    remove_own_verbs();
    okzip_shellmenu::context_menu::install(&shell_menu()).map_err(|e| e.to_string())?;
    notify();
    Ok(())
}

pub fn unregister_context_menu() -> R {
    remove_own_verbs();
    okzip_shellmenu::context_menu::uninstall(okzip_shellmenu::MENU_ID).map_err(|e| e.to_string())?;
    notify();
    Ok(())
}

fn definition_file() -> Option<std::path::PathBuf> {
    let home = okzip_shellmenu::context_menu::windows::data_home().ok()?;
    Some(okzip_shellmenu::context_menu::windows::definition_path(okzip_shellmenu::MENU_ID, &home))
}

/// Whether our context menu is installed for this user.
pub fn context_menu_registered() -> bool {
    definition_file().is_some_and(|p| p.is_file()) || key_exists(&format!(r"Directory\shell\{MENU_VERB}"))
}

/// The sparse package serving the Windows 11 menu, when registered.
pub fn serving_package() -> Option<String> {
    okzip_shellmenu::context_menu::windows::serving_package(okzip_shellmenu::MENU_ID)
}

/// At start-up: when the menu is installed but out of date (exe moved,
/// language changed, the package came or went), install it again. Cheap.
pub fn refresh_context_menu_if_installed() {
    if !context_menu_registered() {
        return;
    }
    let menu = shell_menu();
    if okzip_shellmenu::context_menu::is_current(&menu).ok() != Some(true) {
        let _ = register_context_menu();
    }
}

/// One line for the settings page / `--shell-status`.
pub fn shell_status() -> String {
    let def = definition_file().map(|p| p.display().to_string()).unwrap_or_default();
    format!(
        "registered={} package={} definition={} clsid={}",
        context_menu_registered(),
        serving_package().unwrap_or_else(|| "-".into()),
        def,
        okzip_shellmenu::clsid()
    )
}

/// Register the OKZip.Archive ProgID and offer it for every archive type
/// (Open with list). Windows 10/11 still let the user confirm the default.
pub fn associate() -> R {
    let icon = format!("\"{}\",0", exe());
    set_value(PROGID, None, tr(S::VAssocDesc))?;
    set_value(&format!(r"{PROGID}\DefaultIcon"), None, &icon)?;
    set_value(&format!(r"{PROGID}\shell\open\command"), None, &format!("\"{}\" \"%1\"", exe()))?;
    let app = r"Applications\okzip.exe";
    set_value(app, Some("FriendlyAppName"), "OK Zip")?;
    set_value(&format!(r"{app}\shell\open\command"), None, &format!("\"{}\" \"%1\"", exe()))?;
    for ext in ASSOCIATED_EXTENSIONS {
        set_value(&format!(r".{ext}\OpenWithProgids"), Some(PROGID), "")?;
        set_value(&format!(r"{app}\SupportedTypes"), Some(&format!(".{ext}")), "")?;
    }
    notify();
    Ok(())
}

pub fn unassociate() -> R {
    for ext in ASSOCIATED_EXTENSIONS {
        delete_value(&format!(r".{ext}\OpenWithProgids"), PROGID);
    }
    delete_tree(PROGID);
    delete_tree(r"Applications\okzip.exe");
    notify();
    Ok(())
}
