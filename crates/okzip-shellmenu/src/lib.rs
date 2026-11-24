//! OK Zip's right-click menu in the system file manager, described once and
//! installed through [`context_menu`] (MIT, <https://github.com/hivepaas/context-menu-rs>).
//!
//! * Windows 10, and Windows 11's "Show more options": verbs under
//!   `HKCU\Software\Classes`, written by [`context_menu::install`] when no
//!   package serves the menu.
//! * Windows 11's own (top-level) menu: the prebuilt `context-menu-windows`
//!   `IExplorerCommand` DLL, shipped as `context-menu-ok-zip.dll`
//!   ([`native_dll_name`]) next to `okzip.exe` and declared by OK Zip's
//!   sparse package ([`windows_sparse_manifest`]). With the package
//!   registered, `install` removes the verbs and the DLL serves both menus,
//!   reading the definition `install` writes to
//!   `%LOCALAPPDATA%\context-menu-rs\ok-zip.menu`.
//! * macOS Finder (not built yet): the same [`menu`] with macOS icons; the
//!   Finder Sync extension (`context_menu::macos::FinderExtension`) will read
//!   it. Nothing here is Windows-only, so the future AppKit app reuses it.
//!
//! Items (same as the v0.2 "OK Zip" submenu):
//! * archives: 打开 · 解压到当前文件夹 · 解压到同名文件夹 · 测试 · 压缩为 ZIP · 压缩为 7Z
//! * any other file, and folders: 压缩为 ZIP · 压缩为 7Z

use std::path::Path;

pub use context_menu;
use context_menu::{Count, Item, Menu};

/// The menu's id: names the definition file, the registry verbs, the DLL
/// (`context-menu-ok-zip.dll`) and its COM class.
pub const MENU_ID: &str = "ok-zip";

/// The sparse package's identity name (see `installer/sparse`).
pub const PACKAGE_NAME: &str = "OKZip.ShellMenu";

/// The menu's labels, in the user's language.
#[derive(Clone, Debug)]
pub struct Labels {
    pub title: String,
    pub open: String,
    pub extract_here: String,
    pub extract_smart: String,
    pub test: String,
    pub compress_zip: String,
    pub compress_7z: String,
}

impl Labels {
    pub fn zh() -> Self {
        Labels {
            title: "OK Zip".into(),
            open: "打开".into(),
            extract_here: "解压到当前文件夹".into(),
            extract_smart: "解压到同名文件夹".into(),
            test: "测试".into(),
            compress_zip: "压缩为 ZIP".into(),
            compress_7z: "压缩为 7Z".into(),
        }
    }
    pub fn en() -> Self {
        Labels {
            title: "OK Zip".into(),
            open: "Open".into(),
            extract_here: "Extract here".into(),
            extract_smart: "Extract to folder".into(),
            test: "Test".into(),
            compress_zip: "Compress to ZIP".into(),
            compress_7z: "Compress to 7Z".into(),
        }
    }
}

/// Icons per item, in the platform's form (Windows: `C:\...\okzip.exe,-201`;
/// macOS: an SF Symbol name such as `archivebox`). `None` = no icon.
#[derive(Clone, Debug, Default)]
pub struct Icons {
    pub menu: Option<String>,
    pub open: Option<String>,
    pub extract: Option<String>,
    pub test: Option<String>,
    pub compress: Option<String>,
}

impl Icons {
    /// Icons from `okzip.exe`'s own resources, by resource id (a negative
    /// index): the app icon (1) and the toolbar icons (201 open, 202
    /// extract, 203 new, 206 test).
    pub fn windows_exe(program: &Path) -> Self {
        let p = program.display();
        let r = |id: i32| Some(format!("{p},-{id}"));
        Icons { menu: r(1), open: r(201), extract: r(202), test: r(206), compress: r(203) }
    }
    /// SF Symbols, for the future Finder extension.
    pub fn macos() -> Self {
        let s = |n: &str| Some(n.to_string());
        Icons {
            menu: s("archivebox"),
            open: s("doc.zipper"),
            extract: s("arrow.down.doc"),
            test: s("checkmark.seal"),
            compress: s("archivebox.fill"),
        }
    }
}

/// The command line each item runs is `program <args...> <paths...>`; the
/// app's CLI (see `app/src/win/mod.rs`) takes several paths per run, and
/// `--gather` merges the one-run-per-file of the classic menu.
pub fn menu(program: &Path, labels: &Labels, icons: &Icons, archive_extensions: &[&str]) -> Menu {
    let item = |id: &str, label: &str, icon: &Option<String>, args: &[&str]| {
        let mut it = Item::new(id, label, program).args(args.iter().copied());
        if let Some(i) = icon {
            it = it.icon(i.clone());
        }
        it
    };
    let archives = |it: Item| it.extensions(archive_extensions.iter().copied());
    let mut m = Menu::new(MENU_ID, labels.title.clone()).submenu(true);
    if let Some(i) = &icons.menu {
        m = m.icon(i.clone());
    }
    m.item(archives(item("open", &labels.open, &icons.open, &[])).count(Count::One))
        .item(archives(item("extract-here", &labels.extract_here, &icons.extract, &["x"])))
        .item(archives(item("extract-smart", &labels.extract_smart, &icons.extract, &["xs"])))
        .item(archives(item("test", &labels.test, &icons.test, &["t"])))
        .item(item("compress-zip", &labels.compress_zip, &icons.compress, &["a", "--format", "zip", "--quick", "--gather"]).folders(true))
        .item(item("compress-7z", &labels.compress_7z, &icons.compress, &["a", "--format", "7z", "--quick", "--gather"]).folders(true))
}

/// File name of the Windows 11 menu DLL next to `okzip.exe`.
pub fn native_dll_name() -> String {
    context_menu::windows::native_name(MENU_ID)
}

/// COM CLSID the DLL serves (derived from [`MENU_ID`]).
pub fn clsid() -> String {
    context_menu::windows::clsid(MENU_ID)
}

/// `AppxManifest.xml` of OK Zip's sparse package (package with external
/// location = the install folder). `publisher` must equal the signing
/// certificate's subject exactly (CI: `CN=OK Zip Dev`, self-signed; replace
/// with a CA/Store certificate's subject later). `version` is `a.b.c.d`.
/// Logos are read from `assets\` in the install folder.
pub fn windows_sparse_manifest(version: &str, publisher: &str) -> String {
    let extensions: String = context_menu::windows::package_extensions(MENU_ID).lines().map(|l| format!("        {l}\n")).collect();
    let publisher = xml_escape(publisher);
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
  xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
  xmlns:desktop5="http://schemas.microsoft.com/appx/manifest/desktop/windows10/5"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  xmlns:com="http://schemas.microsoft.com/appx/manifest/com/windows10"
  IgnorableNamespaces="uap uap10 desktop4 desktop5 rescap com">
  <Identity Name="{PACKAGE_NAME}" Publisher="{publisher}" Version="{version}" ProcessorArchitecture="neutral" />
  <Properties>
    <DisplayName>OK Zip</DisplayName>
    <PublisherDisplayName>OK Zip</PublisherDisplayName>
    <Logo>assets\logo50.png</Logo>
    <uap10:AllowExternalContent>true</uap10:AllowExternalContent>
  </Properties>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
    <rescap:Capability Name="unvirtualizedResources" />
  </Capabilities>
  <Applications>
    <Application Id="OKZip" Executable="okzip.exe" uap10:TrustLevel="mediumIL" uap10:RuntimeBehavior="win32App">
      <uap:VisualElements AppListEntry="none" DisplayName="OK Zip" Description="OK Zip" BackgroundColor="transparent" Square150x150Logo="assets\logo150.png" Square44x44Logo="assets\logo44.png" />
      <Extensions>
{extensions}      </Extensions>
    </Application>
  </Applications>
</Package>
"#
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use context_menu::Chosen;

    const EXTS: &[&str] = &["zip", "7z", "rar", "tar", "gz", "tgz", "001"];

    fn shown(m: &Menu, chosen: &[Chosen<'_>]) -> Vec<String> {
        m.items.iter().filter(|i| i.applies_to.accepts(chosen)).map(|i| i.label.clone()).collect()
    }

    fn m() -> Menu {
        let exe = if cfg!(windows) { Path::new(r"C:\Program Files\OK Zip\okzip.exe") } else { Path::new("/opt/okzip/okzip") };
        menu(exe, &Labels::zh(), &Icons::default(), EXTS)
    }

    #[test]
    fn valid() {
        m().validate().unwrap();
        assert!(m().submenu);
        assert_eq!(m().title, "OK Zip");
    }

    #[test]
    fn archive_gets_six() {
        let f = [Chosen { name: "a.zip", folder: false }];
        assert_eq!(shown(&m(), &f), ["打开", "解压到当前文件夹", "解压到同名文件夹", "测试", "压缩为 ZIP", "压缩为 7Z"]);
        let f = [Chosen { name: "B.TAR.GZ", folder: false }];
        assert_eq!(shown(&m(), &f).len(), 6);
    }

    #[test]
    fn file_and_folder_get_two() {
        for f in [[Chosen { name: "说明.txt", folder: false }], [Chosen { name: "文档", folder: true }]] {
            assert_eq!(shown(&m(), &f), ["压缩为 ZIP", "压缩为 7Z"]);
        }
    }

    #[test]
    fn several_archives_no_open() {
        let f = [Chosen { name: "a.zip", folder: false }, Chosen { name: "b.7z", folder: false }];
        assert_eq!(shown(&m(), &f).len(), 5);
        let f = [Chosen { name: "a.zip", folder: false }, Chosen { name: "x", folder: true }];
        assert_eq!(shown(&m(), &f), ["压缩为 ZIP", "压缩为 7Z"]);
    }

    #[test]
    fn definition_roundtrip() {
        let d = m().to_definition();
        assert_eq!(Menu::from_definition(&d).unwrap(), m());
    }

    #[test]
    fn manifest() {
        let x = windows_sparse_manifest("0.3.1.0", "CN=OK Zip Dev");
        assert!(x.contains(r#"Publisher="CN=OK Zip Dev""#));
        assert!(x.contains(&clsid()));
        assert!(x.contains("context-menu-ok-zip.dll"));
        assert_eq!(native_dll_name(), "context-menu-ok-zip.dll");
    }
}
