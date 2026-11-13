//! Format detection and archive naming helpers.

use crate::error::{Error, Result};
use crate::types::{ArchiveFormat, ArchiveInfo};
use crate::volume::{open_reader, volume_set};
use std::io::Read;
use std::path::{Path, PathBuf};

const EXT_TABLE: &[(&str, ArchiveFormat)] = &[
    (".tar.gz", ArchiveFormat::TarGz),
    (".tgz", ArchiveFormat::TarGz),
    (".tar.bz2", ArchiveFormat::TarBz2),
    (".tbz2", ArchiveFormat::TarBz2),
    (".tbz", ArchiveFormat::TarBz2),
    (".tar.xz", ArchiveFormat::TarXz),
    (".txz", ArchiveFormat::TarXz),
    (".tar.zst", ArchiveFormat::TarZst),
    (".tar.zstd", ArchiveFormat::TarZst),
    (".tzst", ArchiveFormat::TarZst),
    (".tar", ArchiveFormat::Tar),
    (".zip", ArchiveFormat::Zip),
    (".zipx", ArchiveFormat::Zip),
    (".jar", ArchiveFormat::Zip),
    (".apk", ArchiveFormat::Zip),
    (".7z", ArchiveFormat::SevenZ),
    (".rar", ArchiveFormat::Rar),
    (".gz", ArchiveFormat::Gz),
    (".bz2", ArchiveFormat::Bz2),
    (".xz", ArchiveFormat::Xz),
    (".zst", ArchiveFormat::Zst),
];

/// Extensions the shell integration registers.
pub const ASSOCIATED_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "tzst", "001",
];

/// Guess the format from the file name alone.
pub fn format_from_name(path: &Path) -> Option<ArchiveFormat> {
    let logical = volume_set(path).map(|(base, _)| base).unwrap_or_else(|| path.to_path_buf());
    let name = logical.file_name()?.to_string_lossy().to_lowercase();
    EXT_TABLE.iter().find(|(ext, _)| name.ends_with(ext)).map(|(_, f)| *f)
}

fn first_volume(path: &Path) -> PathBuf {
    volume_set(path)
        .and_then(|(_, v)| v.into_iter().next())
        .unwrap_or_else(|| path.to_path_buf())
}

fn looks_like_tar(block: &[u8]) -> bool {
    if block.len() >= 262 && &block[257..262] == b"ustar" {
        return true;
    }
    // Old v7 tar: verify header checksum.
    if block.len() >= 512 {
        let stored = std::str::from_utf8(&block[148..156])
            .ok()
            .map(|s| s.trim_matches(|c: char| c == '\0' || c == ' '))
            .and_then(|s| u32::from_str_radix(s, 8).ok());
        if let Some(stored) = stored {
            let sum: u32 = block[..512]
                .iter()
                .enumerate()
                .map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u32 })
                .sum();
            return sum == stored && block[0] != 0;
        }
    }
    false
}

fn read_prefix<R: Read>(mut r: R, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        match r.read(&mut buf[got..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => got += k,
        }
    }
    buf.truncate(got);
    buf
}

/// Detect an archive's format from its magic bytes, falling back to the name.
pub fn detect_format(path: &Path) -> Result<ArchiveFormat> {
    let first = first_volume(path);
    let (mut reader, _, _) = open_reader(&first).or_else(|_| open_reader(path))?;
    let head = read_prefix(&mut reader, 512);
    let by_name = format_from_name(path);

    let fmt = if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") || head.starts_with(b"PK\x07\x08") {
        Some(ArchiveFormat::Zip)
    } else if head.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        Some(ArchiveFormat::SevenZ)
    } else if head.starts_with(b"Rar!\x1A\x07") {
        Some(ArchiveFormat::Rar)
    } else if head.starts_with(&[0x1F, 0x8B]) {
        Some(stream_or_tar(path, ArchiveFormat::Gz, ArchiveFormat::TarGz, by_name))
    } else if head.starts_with(b"BZh") {
        Some(stream_or_tar(path, ArchiveFormat::Bz2, ArchiveFormat::TarBz2, by_name))
    } else if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        Some(stream_or_tar(path, ArchiveFormat::Xz, ArchiveFormat::TarXz, by_name))
    } else if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Some(stream_or_tar(path, ArchiveFormat::Zst, ArchiveFormat::TarZst, by_name))
    } else if looks_like_tar(&head) {
        Some(ArchiveFormat::Tar)
    } else {
        None
    };
    match fmt.or(match by_name {
        // Self-extracting or prefixed ZIPs: trust the extension and let the backend scan.
        Some(ArchiveFormat::Zip) => Some(ArchiveFormat::Zip),
        _ => None,
    }) {
        Some(f) => Ok(f),
        None => Err(Error::UnknownFormat),
    }
}

/// For compressed streams, peek inside to see whether it carries a tarball.
fn stream_or_tar(
    path: &Path,
    plain: ArchiveFormat,
    tar: ArchiveFormat,
    by_name: Option<ArchiveFormat>,
) -> ArchiveFormat {
    if by_name == Some(tar) {
        return tar;
    }
    let peek = (|| -> Result<Vec<u8>> {
        let (r, _, _) = open_reader(path)?;
        let dec = crate::formats::tar::decoder(plain, r)?;
        Ok(read_prefix(dec, 512))
    })();
    match peek {
        Ok(block) if looks_like_tar(&block) => tar,
        _ => plain,
    }
}

/// Archive name without volume suffix and archive extension(s):
/// `photos.tar.gz` → `photos`, `a.part1.rar` → `a`, `b.7z.001` → `b`.
pub fn archive_stem(path: &Path) -> String {
    let logical = volume_set(path).map(|(b, _)| b).unwrap_or_else(|| path.to_path_buf());
    let name = logical.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let lower = name.to_lowercase();
    let mut stem_len = name.len();
    if let Some((ext, _)) = EXT_TABLE.iter().find(|(ext, _)| lower.ends_with(ext)) {
        stem_len = name.len() - ext.len();
    } else if let Some(i) = name.rfind('.') {
        if i > 0 {
            stem_len = i;
        }
    }
    let mut stem = name[..stem_len].to_string();
    // a.part1.rar / a.part01.rar
    let ls = stem.to_lowercase();
    if let Some(i) = ls.rfind(".part") {
        if ls[i + 5..].chars().all(|c| c.is_ascii_digit()) && ls.len() > i + 5 {
            stem.truncate(i);
        }
    }
    if stem.is_empty() {
        "archive".to_string()
    } else {
        stem
    }
}

/// Bandizip-style "解压到 名称\": extract into `parent/<stem>`, unless the
/// archive already holds exactly one top-level folder with that very name, in
/// which case extract straight into `parent` (avoids `name\name\...`).
/// If a plain file of that name exists, a unique sibling folder is used.
pub fn smart_extract_dir(info: &ArchiveInfo, parent: &Path) -> PathBuf {
    let stem = archive_stem(&info.path);
    let tops = info.top_level_names();
    let single_root_dir = tops.len() == 1
        && info.entries.iter().any(|e| e.path.contains('/') || (e.is_dir && e.path == tops[0]));
    if single_root_dir && tops[0] == stem {
        return parent.to_path_buf();
    }
    let target = parent.join(&stem);
    if target.exists() && !target.is_dir() {
        crate::util::unique_path(&target)
    } else {
        target
    }
}

/// Default output path for a new archive made from `sources`:
/// one source → `<dir>/<name>.<ext>`, several → `<dir>/<parent folder name>.<ext>`.
pub fn default_archive_path(sources: &[PathBuf], format: ArchiveFormat) -> Option<PathBuf> {
    let first = sources.first()?;
    let dir = first.parent().map(Path::to_path_buf).unwrap_or_default();
    let base = if sources.len() == 1 {
        let name = first.file_name()?.to_string_lossy().to_string();
        if first.is_dir() {
            name
        } else {
            match name.rfind('.') {
                Some(i) if i > 0 => name[..i].to_string(),
                _ => name,
            }
        }
    } else {
        dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "archive".into())
    };
    Some(dir.join(format!("{base}.{}", format.extension())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        assert_eq!(archive_stem(Path::new("/x/photos.tar.gz")), "photos");
        assert_eq!(archive_stem(Path::new("a.part1.rar")), "a");
        assert_eq!(archive_stem(Path::new("Report.ZIP")), "Report");
        assert_eq!(archive_stem(Path::new("v1.2.zip")), "v1.2");
    }

    #[test]
    fn names() {
        assert_eq!(format_from_name(Path::new("a.TGZ")), Some(ArchiveFormat::TarGz));
        assert_eq!(format_from_name(Path::new("a.tar.zst")), Some(ArchiveFormat::TarZst));
        assert_eq!(format_from_name(Path::new("a.zst")), Some(ArchiveFormat::Zst));
    }
}
