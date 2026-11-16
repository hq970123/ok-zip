//! Explorer integration via per-user registry keys (HKCU\Software\Classes):
//! classic context-menu verbs and "Open with" file associations. No admin
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

fn add_verb(base: &str, verb: &str, label: &str, args: &str) -> R {
    let key = format!(r"{base}\shell\{VERB_PREFIX}{verb}");
    set_value(&key, Some("MUIVerb"), label)?;
    set_value(&key, Some("Icon"), &format!("\"{}\",0", exe()))?;
    set_value(&format!(r"{key}\command"), None, &format!("\"{}\" {args}", exe()))
}

/// Archive verbs on every supported extension; compress verbs on files and folders.
pub fn register_context_menu() -> R {
    for ext in ASSOCIATED_EXTENSIONS {
        let base = format!(r"SystemFileAssociations\.{ext}");
        add_verb(&base, "Open", tr(S::VOpen), "\"%1\"")?;
        add_verb(&base, "ExtractHere", tr(S::VExtractHere), "x \"%1\"")?;
        add_verb(&base, "ExtractTo", tr(S::VExtractTo), "xs \"%1\"")?;
    }
    for base in ["*", "Directory"] {
        add_verb(base, "CompressZip", tr(S::VCompressZip), "a --format zip --quick \"%1\"")?;
        add_verb(base, "Compress7z", tr(S::VCompress7z), "a --format 7z --quick \"%1\"")?;
    }
    notify();
    Ok(())
}

pub fn unregister_context_menu() -> R {
    for ext in ASSOCIATED_EXTENSIONS {
        for verb in ["Open", "ExtractHere", "ExtractTo"] {
            delete_tree(&format!(r"SystemFileAssociations\.{ext}\shell\{VERB_PREFIX}{verb}"));
        }
    }
    for base in ["*", "Directory"] {
        for verb in ["CompressZip", "Compress7z"] {
            delete_tree(&format!(r"{base}\shell\{VERB_PREFIX}{verb}"));
        }
    }
    notify();
    Ok(())
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
