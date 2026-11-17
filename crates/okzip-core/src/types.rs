//! Public data types: formats, entries and operation options.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Every container/stream format OK Zip understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
    Rar,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    /// Single-file gzip stream.
    Gz,
    Bz2,
    Xz,
    Zst,
}

impl ArchiveFormat {
    /// All formats, in UI order.
    pub const ALL: [ArchiveFormat; 12] = [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Rar,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarBz2,
        ArchiveFormat::TarXz,
        ArchiveFormat::TarZst,
        ArchiveFormat::Gz,
        ArchiveFormat::Bz2,
        ArchiveFormat::Xz,
        ArchiveFormat::Zst,
    ];

    /// Formats OK Zip can create.
    pub const WRITABLE: [ArchiveFormat; 7] = [
        ArchiveFormat::Zip,
        ArchiveFormat::SevenZ,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
        ArchiveFormat::TarZst,
        ArchiveFormat::TarBz2,
    ];

    /// Short display name ("ZIP", "TAR.GZ", ...).
    pub fn name(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "ZIP",
            ArchiveFormat::SevenZ => "7Z",
            ArchiveFormat::Rar => "RAR",
            ArchiveFormat::Tar => "TAR",
            ArchiveFormat::TarGz => "TAR.GZ",
            ArchiveFormat::TarBz2 => "TAR.BZ2",
            ArchiveFormat::TarXz => "TAR.XZ",
            ArchiveFormat::TarZst => "TAR.ZST",
            ArchiveFormat::Gz => "GZ",
            ArchiveFormat::Bz2 => "BZ2",
            ArchiveFormat::Xz => "XZ",
            ArchiveFormat::Zst => "ZST",
        }
    }

    /// Preferred file extension, without the leading dot.
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::SevenZ => "7z",
            ArchiveFormat::Rar => "rar",
            ArchiveFormat::Tar => "tar",
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::TarBz2 => "tar.bz2",
            ArchiveFormat::TarXz => "tar.xz",
            ArchiveFormat::TarZst => "tar.zst",
            ArchiveFormat::Gz => "gz",
            ArchiveFormat::Bz2 => "bz2",
            ArchiveFormat::Xz => "xz",
            ArchiveFormat::Zst => "zst",
        }
    }

    pub fn is_writable(self) -> bool {
        Self::WRITABLE.contains(&self)
    }

    /// True for the tar-based family.
    pub fn is_tar(self) -> bool {
        matches!(
            self,
            ArchiveFormat::Tar
                | ArchiveFormat::TarGz
                | ArchiveFormat::TarBz2
                | ArchiveFormat::TarXz
                | ArchiveFormat::TarZst
        )
    }

    /// True for single-file compressed streams (no container).
    pub fn is_single_stream(self) -> bool {
        matches!(
            self,
            ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst
        )
    }

    /// Supports password protection when creating.
    pub fn supports_password(self) -> bool {
        matches!(self, ArchiveFormat::Zip | ArchiveFormat::SevenZ)
    }

    /// Parse a user-supplied format name (CLI / command line).
    pub fn parse(s: &str) -> Option<ArchiveFormat> {
        Some(match s.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "zip" => ArchiveFormat::Zip,
            "7z" | "7zip" => ArchiveFormat::SevenZ,
            "rar" => ArchiveFormat::Rar,
            "tar" => ArchiveFormat::Tar,
            "tgz" | "tar.gz" | "targz" => ArchiveFormat::TarGz,
            "tbz" | "tbz2" | "tar.bz2" => ArchiveFormat::TarBz2,
            "txz" | "tar.xz" => ArchiveFormat::TarXz,
            "tzst" | "tar.zst" | "tar.zstd" => ArchiveFormat::TarZst,
            "gz" | "gzip" => ArchiveFormat::Gz,
            "bz2" | "bzip2" => ArchiveFormat::Bz2,
            "xz" => ArchiveFormat::Xz,
            "zst" | "zstd" => ArchiveFormat::Zst,
            _ => return None,
        })
    }
}

impl std::fmt::Display for ArchiveFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One item inside an archive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// Archive-relative path with `/` separators, no leading or trailing slash.
    pub path: String,
    pub is_dir: bool,
    /// Uncompressed size (0 when unknown, e.g. bare .xz/.zst streams).
    pub size: u64,
    /// Compressed size when the format records it per entry.
    pub packed_size: Option<u64>,
    /// Modification time, Unix seconds (UTC).
    pub modified: Option<i64>,
    pub crc32: Option<u32>,
    pub encrypted: bool,
    /// Raw attributes: DOS/Windows attribute bits or Unix mode, format dependent.
    pub attributes: Option<u32>,
    /// Compression method label ("Deflate", "LZMA2", ...).
    pub method: Option<String>,
    /// Backend index (position in the archive's own directory).
    pub index: usize,
}

impl Entry {
    /// Last path component.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// Parent folder path ("" for top level).
    pub fn parent(&self) -> &str {
        match self.path.rfind('/') {
            Some(i) => &self.path[..i],
            None => "",
        }
    }
}

/// Result of listing an archive.
#[derive(Clone, Debug)]
pub struct ArchiveInfo {
    pub path: PathBuf,
    pub format: ArchiveFormat,
    pub entries: Vec<Entry>,
    /// Any entry (or the header) is encrypted.
    pub encrypted: bool,
    pub solid: bool,
    /// Number of physical volume files (1 for a normal archive).
    pub volumes: usize,
    /// Size on disk of all volumes together.
    pub archive_size: u64,
}

impl ArchiveInfo {
    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_dir).count()
    }
    pub fn dir_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_dir).count()
    }
    pub fn total_size(&self) -> u64 {
        self.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum()
    }
    /// Compressed size: sum of per-entry sizes, or the archive size when the
    /// format doesn't record them.
    pub fn total_packed(&self) -> u64 {
        let per: Option<u64> =
            self.entries.iter().filter(|e| !e.is_dir).map(|e| e.packed_size).sum();
        per.unwrap_or(self.archive_size)
    }
    /// Distinct first path components.
    pub fn top_level_names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for e in &self.entries {
            let first = e.path.split('/').next().unwrap_or("").to_string();
            if !first.is_empty() && !v.contains(&first) {
                v.push(first);
            }
        }
        v
    }
}

/// What to do when an extracted file already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OverwriteMode {
    #[default]
    Overwrite,
    Skip,
    /// Write to "name (2).ext" instead.
    Rename,
    /// Ask through [`ExtractOptions::on_conflict`] (falls back to Skip).
    Ask,
}

/// Answer to a conflict question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Overwrite,
    Skip,
    Rename,
    /// Apply to this and all later conflicts.
    OverwriteAll,
    SkipAll,
    RenameAll,
    Cancel,
}

/// What the extractor knows about a conflict when it asks [`ExtractOptions::on_conflict`].
#[derive(Clone, Copy, Debug)]
pub struct ConflictInfo<'a> {
    /// The existing file that would be replaced.
    pub target: &'a Path,
    /// The entry's path inside the archive.
    pub entry_path: &'a str,
    /// Uncompressed size of the incoming entry, when the format records it.
    pub new_size: Option<u64>,
    /// Modification time (Unix seconds) of the incoming entry.
    pub new_modified: Option<i64>,
}

pub type ConflictCallback = dyn Fn(&ConflictInfo<'_>) -> ConflictChoice + Send + Sync;

#[derive(Clone, Default)]
pub struct OpenOptions {
    pub password: Option<String>,
}

#[derive(Clone, Default)]
pub struct ExtractOptions {
    pub password: Option<String>,
    pub overwrite: OverwriteMode,
    pub on_conflict: Option<Arc<ConflictCallback>>,
    /// Only extract these entry paths (a folder path selects its subtree).
    /// `None` extracts everything.
    pub selection: Option<Vec<String>>,
    /// Remove this folder prefix from output paths (e.g. when extracting a
    /// selection from inside a sub-folder).
    pub strip_prefix: Option<String>,
    /// Restore modification times (default true via `ExtractOptions::new`).
    pub preserve_mtime: bool,
}

impl ExtractOptions {
    pub fn new() -> Self {
        ExtractOptions { preserve_mtime: true, ..Default::default() }
    }
    pub fn password(mut self, pw: Option<String>) -> Self {
        self.password = pw;
        self
    }
    pub fn overwrite(mut self, m: OverwriteMode) -> Self {
        self.overwrite = m;
        self
    }
}

impl std::fmt::Debug for ExtractOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtractOptions")
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("overwrite", &self.overwrite)
            .field("selection", &self.selection)
            .field("strip_prefix", &self.strip_prefix)
            .finish()
    }
}

/// ZIP encryption flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ZipEncryption {
    #[default]
    Aes256,
    /// Legacy PKWARE ZipCrypto (weak; for compatibility only).
    ZipCrypto,
}

#[derive(Clone, Debug)]
pub struct CreateOptions {
    pub format: ArchiveFormat,
    /// 0 (store) ..= 9 (ultra).
    pub level: u32,
    pub password: Option<String>,
    pub zip_encryption: ZipEncryption,
    /// 7z: also encrypt file names (header encryption).
    pub encrypt_headers: bool,
    /// 7z: solid compression.
    pub solid: bool,
    /// Split into volumes of this many bytes (`name.ext.001`, `.002`, ...).
    pub split_size: Option<u64>,
}

impl Default for CreateOptions {
    fn default() -> Self {
        CreateOptions {
            format: ArchiveFormat::Zip,
            level: 5,
            password: None,
            zip_encryption: ZipEncryption::Aes256,
            encrypt_headers: false,
            solid: true,
            split_size: None,
        }
    }
}

impl CreateOptions {
    pub fn new(format: ArchiveFormat) -> Self {
        CreateOptions { format, ..Default::default() }
    }
}

/// Statistics returned by extract/create.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpStats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub skipped: u64,
    /// Output files written (archive volumes for create, extracted files otherwise).
    pub outputs: Vec<PathBuf>,
}

/// Result of an integrity test.
#[derive(Clone, Debug, Default)]
pub struct TestReport {
    pub files_tested: u64,
    pub bytes_tested: u64,
    /// (entry path, problem description)
    pub errors: Vec<(String, String)>,
}

impl TestReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Edits applied by [`crate::modify`].
#[derive(Clone, Debug, Default)]
pub struct ModifyOps {
    /// Files/folders on disk to add.
    pub add: Vec<PathBuf>,
    /// Archive folder to add them into ("" = root).
    pub add_into: String,
    /// Entry paths to delete (a folder deletes its subtree).
    pub delete: Vec<String>,
}
