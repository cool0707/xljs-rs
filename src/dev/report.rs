//! What the development mode makes of a failed run: the error, where it
//! happened, and a request for the AI chat.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::project::{Project, read_text};
use crate::t;

/// A place in a file of the script.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Location {
    /// Relative to the script's folder, with slashes.
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// "at f (file:line:col)" of a stack trace.
static FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*at (?:.*\()?(.+?):(\d+):(\d+)\)?\s*$").unwrap());
/// "file:line:col: message" of a syntax error.
static SYNTAX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+?\.[A-Za-z]+):(\d+):(\d+): ").unwrap());

/// The file of a location as the script's folder sees it; None outside it.
fn relative(dir: &Path, file: &str) -> Option<String> {
    let p = Path::new(file);
    let abs = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
    let rel = abs.strip_prefix(dir).ok()?.to_string_lossy().replace('\\', "/");
    if rel.split('/').any(|x| x == "node_modules" || x == "..") || !abs.is_file() {
        return None;
    }
    Some(rel)
}

/// The first place in the script's own files named by text (a stack
/// trace or a syntax error).
pub fn find_location(text: &str, dir: &Path) -> Option<Location> {
    for line in text.lines() {
        let c = FRAME.captures(line).or_else(|| SYNTAX.captures(line));
        let Some(c) = c else { continue };
        let Some(file) = relative(dir, &c[1]) else { continue };
        return Some(Location { file, line: c[2].parse().ok()?, column: c[3].parse().ok()? });
    }
    None
}

/// The part of a run's stderr that tells why it ended: from the last line
/// that is not indented (the error, or the stop notice) to the end.
pub fn last_error(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.trim_end().lines().collect();
    let start = lines.iter().rposition(|l| !l.is_empty() && !l.starts_with(char::is_whitespace)).unwrap_or(0);
    // A heading line ("Syntax error:", "It was running here:") belongs to it.
    let start = if start > 0 && lines[start - 1].trim_end().ends_with(':') && !lines[start - 1].starts_with(char::is_whitespace) { start - 1 } else { start };
    lines[start..].join("\n")
}

/// Lines around line (1-based) of text, numbered, the line marked.
pub fn excerpt(text: &str, line: u32, around: u32) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let line = line.max(1) as usize;
    let from = line.saturating_sub(around as usize).max(1);
    let to = (line + around as usize).min(lines.len());
    let width = to.to_string().len();
    let mut out = String::new();
    for n in from..=to {
        let mark = if n == line { ">" } else { " " };
        out.push_str(&format!("{mark}{n:>width$}| {}\n", lines[n - 1]));
    }
    out
}

/// The code fence language of a file.
fn fence(file: &str) -> &'static str {
    match super::project::ext_of(file).as_str() {
        "ts" | "mts" => "ts",
        "html" | "htm" => "html",
        "css" => "css",
        "json" => "json",
        _ => "js",
    }
}

/// The request for the AI chat after a failed or stopped run.
pub fn prompt(p: &Project, stopped: bool, message: &str, loc: Option<&Location>) -> String {
    let file = loc.map(|l| l.file.clone()).unwrap_or_else(|| p.entry_name());
    let mut s = if stopped { t!("dev.promptStopped", file) } else { t!("dev.promptError", file) };
    s.push_str("\n\n");
    if !stopped && !message.is_empty() {
        s.push_str(&format!("{}\n```\n{}\n```\n\n", t!("dev.promptMessage"), message.trim_end()));
    }
    if let Some(l) = loc
        && let Ok(text) = read_text(&p.dir.join(&l.file))
    {
        s.push_str(&format!("{}\n```{}\n{}```\n\n", t!("dev.promptAround", l.file, l.line), fence(&l.file), excerpt(&text, l.line, 8)));
    }
    s.push_str(&t!("dev.promptAsk", file));
    s
}

/// The instructions for an AI chat that does not know xljs yet
/// (docs/ai/prompt.md) and the script so far.
pub fn intro(p: Option<&Project>) -> String {
    let mut s = crate::dev::AI_PROMPT.trim_end().to_string();
    if let Some(p) = p {
        let name = p.entry_name();
        if let Ok(text) = read_text(&p.entry)
            && !text.trim().is_empty()
        {
            s.push_str(&format!("\n\n---\n\n{}\n```{}\n{}\n```\n", t!("dev.introCurrent", name), fence(&name), text.trim_end()));
        }
    }
    s
}
