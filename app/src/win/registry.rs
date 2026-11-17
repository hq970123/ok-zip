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

/// ECF_SEPARATORBEFORE
const SEPARATOR_BEFORE: u32 = 0x20;

fn set_dword(path: &str, name: &str, value: u32) -> R {
    unsafe {
        let p = wide(&format!(r"{CLASSES}\{path}"));
        let mut key: HKEY = null_mut();
        let rc = RegCreateKeyExW(HKEY_CURRENT_USER, p.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, null(), &mut key, null_mut());
        if rc != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyEx({path}) = {rc}"));
        }
        let n = wide(name);
        let rc = RegSetValueExW(key, n.as_ptr(), 0, REG_DWORD, &value as *const u32 as *const u8, 4);
        RegCloseKey(key);
        if rc != ERROR_SUCCESS {
            return Err(format!("RegSetValueEx({path}) = {rc}"));
        }
    }
    Ok(())
}

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

struct Verb {
    key: &'static str,
    label: S,
    args: &'static str,
    separator: bool,
}

const fn verb(key: &'static str, label: S, args: &'static str, separator: bool) -> Verb {
    Verb { key, label, args, separator }
}

/// Archive files: the full submenu. `--gather` lets one process collect every
/// file of a multi-selection (Explorer starts one process per item).
const ARCHIVE_VERBS: [Verb; 6] = [
    verb("01Open", S::VOpen, "\"%1\"", false),
    verb("02ExtractHere", S::VExtractHere, "x \"%1\"", false),
    verb("03ExtractSmart", S::VExtractTo, "xs \"%1\"", false),
    verb("04Test", S::VTest, "t \"%1\"", false),
    verb("05CompressZip", S::VCompressZip, "a --format zip --quick --gather \"%1\"", true),
    verb("06Compress7z", S::VCompress7z, "a --format 7z --quick --gather \"%1\"", false),
];
/// Any other file or folder: compress only.
const COMPRESS_VERBS: [Verb; 2] = [
    verb("01CompressZip", S::VCompressZip, "a --format zip --quick --gather \"%1\"", false),
    verb("02Compress7z", S::VCompress7z, "a --format 7z --quick --gather \"%1\"", false),
];

fn icon_value() -> String {
    format!("\"{}\",0", exe())
}

/// `<base>\shell\OKZip` = cascading "OK Zip" entry pointing at `submenu`.
fn add_cascade(base: &str, submenu: &str, applies_to: Option<&str>) -> R {
    let key = format!(r"{base}\shell\{MENU_VERB}");
    delete_tree(&key);
    set_value(&key, Some("MUIVerb"), tr(S::VMenu))?;
    set_value(&key, Some("Icon"), &icon_value())?;
    set_value(&key, Some("ExtendedSubCommandsKey"), submenu)?;
    // Keep the entry visible for any number of selected items (the default
    // "Document" model hides static verbs above 15 items).
    set_value(&key, Some("MultiSelectModel"), "Player")?;
    if let Some(a) = applies_to {
        set_value(&key, Some("AppliesTo"), a)?;
    }
    Ok(())
}

fn add_submenu(submenu: &str, verbs: &[Verb]) -> R {
    delete_tree(submenu);
    for v in verbs {
        let key = format!(r"{submenu}\shell\{}", v.key);
        set_value(&key, Some("MUIVerb"), tr(v.label))?;
        set_value(&key, Some("Icon"), &icon_value())?;
        set_value(&key, Some("MultiSelectModel"), "Player")?;
        if v.separator {
            set_dword(&key, "CommandFlags", SEPARATOR_BEFORE)?;
        }
        set_value(&format!(r"{key}\command"), None, &format!("\"{}\" {}", exe(), v.args))?;
    }
    Ok(())
}

/// AQS condition matching every file that is NOT one of our archive types, so
/// archives show exactly one "OK Zip" submenu (the archive one).
fn non_archive_condition() -> String {
    let parts: Vec<String> = ASSOCIATED_EXTENSIONS.iter().map(|e| format!("System.FileExtension:=.{e}")).collect();
    format!("NOT ({})", parts.join(" OR "))
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

/// One cascading "OK Zip" submenu in the Explorer context menu (per user):
/// archives get 打开 / 解压到当前文件夹 / 解压到同名文件夹 / 测试 / 压缩为 ZIP / 压缩为 7Z,
/// other files and folders get the two compress commands.
pub fn register_context_menu() -> R {
    remove_legacy_verbs();
    add_submenu(ARCHIVE_MENU, &ARCHIVE_VERBS)?;
    add_submenu(COMPRESS_MENU, &COMPRESS_VERBS)?;
    for ext in ASSOCIATED_EXTENSIONS {
        add_cascade(&format!(r"SystemFileAssociations\.{ext}"), ARCHIVE_MENU, None)?;
    }
    add_cascade("*", COMPRESS_MENU, Some(&non_archive_condition()))?;
    add_cascade("Directory", COMPRESS_MENU, None)?;
    notify();
    Ok(())
}

pub fn unregister_context_menu() -> R {
    remove_legacy_verbs();
    for ext in ASSOCIATED_EXTENSIONS {
        delete_tree(&format!(r"SystemFileAssociations\.{ext}\shell\{MENU_VERB}"));
    }
    for base in ["*", "Directory"] {
        delete_tree(&format!(r"{base}\shell\{MENU_VERB}"));
    }
    delete_tree(ARCHIVE_MENU);
    delete_tree(COMPRESS_MENU);
    notify();
    Ok(())
}

/// Whether our context-menu entries are currently registered for this user.
pub fn context_menu_registered() -> bool {
    key_exists(&format!(r"Directory\shell\{MENU_VERB}")) || key_exists(&format!(r"SystemFileAssociations\.zip\shell\{MENU_VERB}"))
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
