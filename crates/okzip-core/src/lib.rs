//! # okzip-core
//!
//! Platform-independent archive engine behind OK Zip. It knows nothing about
//! any UI: every long operation takes a [`Progress`] (throttled callback +
//! cancellation flag) and returns a plain [`Result`].
//!
//! | Format            | List/Extract/Test | Create | Password            |
//! |-------------------|-------------------|--------|---------------------|
//! | ZIP               | yes               | yes    | AES-256, ZipCrypto  |
//! | 7Z                | yes               | yes    | AES-256 (+headers)  |
//! | RAR (feature)     | yes               | no     | yes (read)          |
//! | TAR(.GZ/.BZ2/.XZ/.ZST) | yes          | yes    | –                   |
//! | GZ/BZ2/XZ/ZST     | yes               | no     | –                   |
//!
//! Split volumes (`.001`, `.002`, ...) are read for every format and written
//! for every writable format (raw byte split, 7-Zip/Bandizip compatible).

mod detect;
mod error;
mod formats;
mod ops;
mod progress;
mod types;
mod util;
mod volume;

pub use detect::{archive_stem, default_archive_path, detect_format, format_from_name, smart_extract_dir, ASSOCIATED_EXTENSIONS};
pub use error::{Error, Result};
pub use ops::{create, extract, list, modify, test};
pub use progress::{Progress, ProgressCallback, ProgressInfo};
pub use types::*;
pub use util::{normalize_entry_path, sanitize_rel_path, unique_path};
pub use volume::{volume_set, MultiFileReader};

/// Whether this build can read RAR archives.
pub const RAR_SUPPORTED: bool = cfg!(feature = "rar");

/// Parse sizes like "700M", "4.5g", "100k", "1048576" (binary units).
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase().replace(' ', "");
    let s = s.trim_end_matches('b').trim_end_matches('i');
    let (num, mult) = match s.chars().last()? {
        'k' => (&s[..s.len() - 1], 1024u64),
        'm' => (&s[..s.len() - 1], 1024 * 1024),
        'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        't' => (&s[..s.len() - 1], 1024u64.pow(4)),
        _ => (s, 1),
    };
    let v: f64 = num.parse().ok()?;
    if v <= 0.0 {
        return None;
    }
    Some((v * mult as f64) as u64)
}

/// Human readable size: "1.5 MB" style (binary units).
pub fn format_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[u])
    } else {
        format!("{v:.2} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::parse_size("10m"), Some(10 * 1024 * 1024));
        assert_eq!(super::parse_size("1.5K"), Some(1536));
        assert_eq!(super::parse_size("123"), Some(123));
        assert_eq!(super::parse_size("x"), None);
        assert_eq!(super::format_size(1536), "1.50 KB");
    }
}
