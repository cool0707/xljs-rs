use std::io;

use crate::Error;

use super::{current, message, t};

/// Translates well-known English library messages in s with the active
/// locale's translation rules.
pub fn text(s: &str) -> String {
    let loc = current();
    if s.is_empty() {
        return String::new();
    }
    let mut s = s.to_string();
    for (re, rep) in loc.rules() {
        if re.is_match(&s) {
            s = re.replace_all(&s, rep).into_owned();
        }
    }
    s
}

/// Describes the OS-level cause of err.
fn reason(err: &io::Error) -> String {
    if cfg!(windows)
        && let Some(32 | 33) = err.raw_os_error()
    {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        return t("os.reason.inUse", &[]);
    }
    let key = match err.kind() {
        io::ErrorKind::NotFound => "os.reason.notExist",
        io::ErrorKind::PermissionDenied => "os.reason.permission",
        io::ErrorKind::AlreadyExists => "os.reason.exist",
        io::ErrorKind::NotADirectory => "os.reason.notDir",
        io::ErrorKind::IsADirectory => "os.reason.isDir",
        io::ErrorKind::DirectoryNotEmpty => "os.reason.notEmpty",
        io::ErrorKind::StorageFull => "os.reason.noSpace",
        _ => {
            // Other error texts (on Windows they come from the system in
            // its own language) are passed through the translation rules.
            return text(&os_text(err));
        }
    };
    t(key, &[])
}

/// The text of an OS error without Rust's " (os error N)" suffix.
pub(crate) fn os_text(err: &io::Error) -> String {
    let s = err.to_string();
    match s.rfind(" (os error ") {
        Some(i) if s.ends_with(')') => s[..i].to_string(),
        _ => s,
    }
}

fn op_name(op: &str) -> String {
    let key = format!("os.op.{op}");
    let m = message(&key);
    if m != key { m } else { op.to_string() }
}

/// The user-facing message for err in the active language.
pub fn error(err: &Error) -> String {
    match err {
        Error::Path { op, path, source } => t("os.error", &[path.clone(), op_name(op), reason(source)]),
        Error::Link { op, old, new, source } => {
            t("os.error", &[format!("{old} → {new}"), op_name(op), reason(source)])
        }
        Error::Context { prefix, inner } => format!("{}: {}", text(prefix), error(inner)),
        other => text(&other.to_string()),
    }
}
