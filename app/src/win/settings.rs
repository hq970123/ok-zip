//! Persisted user settings: `%APPDATA%\OKZip\settings.json` (serde JSON).
//!
//! Loaded once at start-up; every change is written back immediately.
//! `OKZIP_SETTINGS` overrides the file path (used by tests / screenshots).

use okzip_core::{ArchiveFormat, OverwriteMode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const MAX_RECENT: usize = 10;

/// What the toolbar "Extract" button (and the Enter-less default action) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtractTarget {
    /// Next to the archive.
    CurrentFolder,
    /// Into "<name>\" unless the archive already has a single top folder.
    SubfolderSmart,
    /// Show the extract dialog.
    #[default]
    Ask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverwritePolicy {
    #[default]
    Ask,
    Overwrite,
    Skip,
    Rename,
}

impl OverwritePolicy {
    pub fn mode(self) -> OverwriteMode {
        match self {
            OverwritePolicy::Ask => OverwriteMode::Ask,
            OverwritePolicy::Overwrite => OverwriteMode::Overwrite,
            OverwritePolicy::Skip => OverwriteMode::Skip,
            OverwritePolicy::Rename => OverwriteMode::Rename,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Auto,
    Zh,
    En,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    // extraction
    pub extract_target: ExtractTarget,
    pub overwrite: OverwritePolicy,
    pub open_folder_after_extract: bool,
    // compression
    /// Extension-style id: "zip", "7z", "tar", "tar.gz", ...
    pub create_format: String,
    pub create_level: u32,
    // window
    pub remember_window: bool,
    /// Physical pixels (as returned by GetWindowPlacement).
    pub window: Option<WindowRect>,
    /// List column widths in 96-DPI units.
    pub column_widths: Vec<i32>,
    /// Tree/list splitter position in 96-DPI units.
    pub splitter: i32,
    pub show_tree: bool,
    pub recent: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: Language::Auto,
            extract_target: ExtractTarget::Ask,
            overwrite: OverwritePolicy::Ask,
            open_folder_after_extract: true,
            create_format: "zip".into(),
            create_level: 5,
            remember_window: true,
            window: None,
            column_widths: vec![],
            splitter: 230,
            show_tree: true,
            recent: vec![],
        }
    }
}

impl Settings {
    pub fn format(&self) -> ArchiveFormat {
        ArchiveFormat::parse(&self.create_format)
            .filter(|f| ArchiveFormat::WRITABLE.contains(f))
            .unwrap_or(ArchiveFormat::Zip)
    }
    pub fn set_format(&mut self, f: ArchiveFormat) {
        self.create_format = f.extension().to_string();
    }
    pub fn level(&self) -> u32 {
        self.create_level.min(9)
    }
    pub fn add_recent(&mut self, p: &Path) {
        let key = p.to_string_lossy().to_lowercase();
        self.recent.retain(|r| r.to_string_lossy().to_lowercase() != key);
        self.recent.insert(0, p.to_path_buf());
        self.recent.truncate(MAX_RECENT);
    }
}

static SETTINGS: OnceLock<Mutex<Settings>> = OnceLock::new();

pub fn settings_path() -> PathBuf {
    if let Some(p) = std::env::var_os("OKZIP_SETTINGS") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("OKZip").join("settings.json")
}

fn load_from_disk() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => {
            // Tolerate a UTF-8 BOM (files edited by Notepad / PowerShell).
            let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            serde_json::from_slice(b).unwrap_or_default()
        }
        Err(_) => Settings::default(),
    }
}

fn cell() -> &'static Mutex<Settings> {
    SETTINGS.get_or_init(|| Mutex::new(load_from_disk()))
}

/// A snapshot of the current settings.
pub fn get() -> Settings {
    cell().lock().map(|g| g.clone()).unwrap_or_default()
}

/// Change the settings and write them to disk.
pub fn update(f: impl FnOnce(&mut Settings)) {
    let snapshot = {
        let Ok(mut g) = cell().lock() else { return };
        f(&mut g);
        g.clone()
    };
    save(&snapshot);
}

fn save(s: &Settings) {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(s) {
        // write-then-rename so a crash never leaves a truncated file
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}
