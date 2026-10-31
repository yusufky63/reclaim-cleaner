use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the path is protected: {0}")]
    Protected(String),
    #[error("unknown rule: {0}")]
    UnknownRule(String),
    #[error("invalid rule {id}: {reason}")]
    InvalidRule { id: String, reason: String },
    #[error("invalid path pattern: {0}")]
    Pattern(String),
    #[error("the job was cancelled")]
    Cancelled,
    #[error("administrator rights are required")]
    NeedsAdmin,
    #[error("{0}")]
    Windows(String),
    /// The feature has no implementation on this operating system yet.
    #[error("not available on this platform yet: {0}")]
    Unsupported(&'static str),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Stable machine-readable code the UI maps to a translated message.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied => {
                "access_denied"
            }
            Error::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
                "not_found"
            }
            Error::Io { .. } => "io",
            Error::Protected(_) => "protected",
            Error::UnknownRule(_) => "unknown_rule",
            Error::InvalidRule { .. } => "invalid_rule",
            Error::Pattern(_) => "pattern",
            Error::Cancelled => "cancelled",
            Error::NeedsAdmin => "needs_admin",
            Error::Windows(_) => "windows",
            Error::Unsupported(_) => "unsupported",
            Error::Other(_) => "other",
        }
    }
}

#[cfg(windows)]
impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Error::Windows(e.message())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Other(e.to_string())
    }
}

/// Error shape sent to the UI: a code for translation plus the raw message for details.
#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    pub code: &'static str,
    pub message: String,
}

impl From<&Error> for ErrorPayload {
    fn from(e: &Error) -> Self {
        ErrorPayload {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
