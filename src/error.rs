//! The error type of xljs.

use std::fmt;
use std::io;

use crate::engine::{BundleError, ScriptError};

/// An error of xljs. `Display` gives the untranslated (library) text; use
/// [`crate::i18n::error`] for the message shown to users.
#[derive(Debug)]
pub enum Error {
    /// A message from the locale files (already localized).
    Localized { key: String, msg: String },
    /// A failed file operation (like Go's fs.PathError).
    Path { op: String, path: String, source: io::Error },
    /// A failed operation on two paths (rename).
    Link { op: String, old: String, new: String, source: io::Error },
    /// An error with context in front ("Sheet1!A1: ...").
    Context { prefix: String, inner: Box<Error> },
    Bundle(BundleError),
    Script(ScriptError),
    /// Any other message (from a library, in English).
    Msg(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn localized(key: &str, msg: String) -> Self {
        Error::Localized { key: key.to_string(), msg }
    }
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }
    pub fn path(op: &str, path: impl Into<String>, source: io::Error) -> Self {
        Error::Path { op: op.to_string(), path: path.into(), source }
    }
    /// Puts prefix in front of the error ("prefix: error").
    pub fn context(self, prefix: impl Into<String>) -> Self {
        Error::Context { prefix: prefix.into(), inner: Box::new(self) }
    }
    /// The message key of a localized error.
    pub fn key(&self) -> Option<&str> {
        match self {
            Error::Localized { key, .. } => Some(key),
            Error::Context { inner, .. } => inner.key(),
            _ => None,
        }
    }
    /// Whether this is (or wraps) a "file not found" error.
    pub fn is_not_found(&self) -> bool {
        match self {
            Error::Path { source, .. } | Error::Link { source, .. } => source.kind() == io::ErrorKind::NotFound,
            Error::Context { inner, .. } => inner.is_not_found(),
            _ => false,
        }
    }
    /// Whether the error names path already (so that callers do not add it).
    pub fn has_path(&self) -> bool {
        match self {
            Error::Path { .. } | Error::Link { .. } => true,
            Error::Context { inner, .. } => inner.has_path(),
            _ => false,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Localized { msg, .. } => f.write_str(msg),
            Error::Path { op, path, source } => {
                write!(f, "{op} {path}: {}", crate::i18n::translate_os_text(source))
            }
            Error::Link { op, old, new, source } => {
                write!(f, "{op} {old} {new}: {}", crate::i18n::translate_os_text(source))
            }
            Error::Context { prefix, inner } => write!(f, "{prefix}: {inner}"),
            Error::Bundle(e) => e.fmt(f),
            Error::Script(e) => e.fmt(f),
            Error::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Msg(crate::i18n::translate_os_text(&e))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Msg(e.to_string())
    }
}
