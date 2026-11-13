//! Error type shared by every format backend.

use std::io;

/// Result alias used throughout `okzip-core`.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while reading or writing an archive.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(io::Error),
    #[error("unrecognized or unsupported archive format")]
    UnknownFormat,
    #[error("operation not supported: {0}")]
    Unsupported(String),
    #[error("a password is required")]
    PasswordRequired,
    #[error("wrong password")]
    WrongPassword,
    #[error("operation cancelled")]
    Cancelled,
    #[error("archive is corrupt: {0}")]
    Corrupt(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// True when retrying with a (different) password could help.
    pub fn needs_password(&self) -> bool {
        matches!(self, Error::PasswordRequired | Error::WrongPassword)
    }
}

/// Marker payload carried inside an `io::Error` so that cancellation survives
/// a round-trip through third-party code that only speaks `io::Result`.
#[derive(Debug)]
pub(crate) struct CancelledMarker;

impl std::fmt::Display for CancelledMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("operation cancelled")
    }
}
impl std::error::Error for CancelledMarker {}

pub(crate) fn cancelled_io() -> io::Error {
    io::Error::other(CancelledMarker)
}

pub(crate) fn is_cancel_io(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<CancelledMarker>())
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if is_cancel_io(&e) {
            return Error::Cancelled;
        }
        Error::Io(e)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        use zip::result::ZipError as Z;
        match e {
            Z::Io(io) => io.into(),
            Z::InvalidPassword => Error::WrongPassword,
            Z::UnsupportedArchive(msg) if msg == Z::PASSWORD_REQUIRED => Error::PasswordRequired,
            Z::UnsupportedArchive(msg) => Error::Unsupported(msg.to_string()),
            Z::CompressionMethodNotSupported(m) => {
                Error::Unsupported(format!("ZIP compression method {m}"))
            }
            Z::InvalidArchive(msg) => Error::Corrupt(msg.to_string()),
            other => Error::Other(other.to_string()),
        }
    }
}

impl From<sevenz_rust2::Error> for Error {
    fn from(e: sevenz_rust2::Error) -> Self {
        use sevenz_rust2::Error as S;
        match e {
            S::PasswordRequired => Error::PasswordRequired,
            S::MaybeBadPassword(_) => Error::WrongPassword,
            S::Io(io, _) | S::FileOpen(io, _) => io.into(),
            S::ChecksumVerificationFailed | S::NextHeaderCrcMismatch => {
                Error::Corrupt("CRC mismatch".into())
            }
            S::BadSignature(_) => Error::UnknownFormat,
            S::UnsupportedCompressionMethod(m) => Error::Unsupported(format!("7z method {m}")),
            S::Unsupported(m) => Error::Unsupported(m.to_string()),
            other => Error::Other(other.to_string()),
        }
    }
}

#[cfg(feature = "rar")]
impl From<unrar::error::UnrarError> for Error {
    fn from(e: unrar::error::UnrarError) -> Self {
        use unrar::error::Code;
        match e.code {
            Code::MissingPassword => Error::PasswordRequired,
            Code::BadPassword => Error::WrongPassword,
            Code::BadData => Error::Corrupt(e.to_string()),
            Code::BadArchive | Code::UnknownFormat => Error::Corrupt(e.to_string()),
            _ => Error::Other(e.to_string()),
        }
    }
}
