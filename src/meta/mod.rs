//! The metadata block at the top of an entry script and access to the
//! files (assets) a script ships with.
//!
//! ```text
//! // ==xljs==
//! // @mode    app            // console (default) | app
//! // @html    ui/index.html  // first page, relative to the script
//! // @assets  ui/            // folders bundled into the exe (repeatable)
//! // @title   売上集計ツール
//! // @size    960x640
//! // @icon    ui/app.ico     // window icon (.ico or .png), bundled too
//! // @lang    ja
//! // ==/xljs==
//! ```
//!
//! Only line comments before the first line of code are examined.

mod assets;

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{Result, err, t};

pub use assets::{AssetFs, DirAssets, MemAssets, covered, valid_path as assets_valid_path};

pub const MODE_CONSOLE: &str = "console";
pub const MODE_APP: &str = "app";

/// The parsed metadata block. Paths are slash-separated and relative to
/// the script directory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub html: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub height: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// "custom" hides the OS title bar of the main window ("" = system).
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "titleBar")]
    pub title_bar: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

static KEY_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@([A-Za-z][\w-]*)(?:\s+(.*))?$").unwrap());
static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+//.*$").unwrap());
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)\s*[xX×]\s*(\d+)$").unwrap());

impl Meta {
    /// Whether the script runs in app mode (opens a main window).
    pub fn is_app(&self) -> bool {
        self.mode == MODE_APP
    }

    /// The files and folders a script ships with: the @assets entries plus
    /// the @html page and the @icon file.
    pub fn asset_roots(&self) -> Vec<String> {
        let mut roots = self.assets.clone();
        for f in [&self.html, &self.icon] {
            if !f.is_empty() && !covered(&roots, f) {
                roots.push(f.clone());
            }
        }
        roots
    }

    /// The script's assets read from the script folder dir. Only paths under
    /// asset_roots are visible, so that running from source serves the same
    /// files as the packed exe.
    pub fn asset_fs(&self, dir: &std::path::Path) -> DirAssets {
        DirAssets::new(dir, self.asset_roots())
    }
}

/// Reads the metadata block of the script at path. A script without a block
/// yields an empty Meta. Warnings (unknown keys) are returned separately;
/// malformed values are errors.
pub fn parse_file(file: &str) -> Result<(Meta, Vec<String>)> {
    let src = std::fs::read(file).map_err(|e| crate::Error::path("open", file, e))?;
    parse(file, &src)
}

/// Reads the metadata block from src; name is used in messages.
pub fn parse(name: &str, src: &[u8]) -> Result<(Meta, Vec<String>)> {
    let mut m = Meta::default();
    let mut warnings = Vec::new();
    let src = src.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(src);
    let text = String::from_utf8_lossy(src);
    let (mut inside, mut done) = (false, false);
    for (i, raw) in text.split('\n').enumerate() {
        let line_no = i + 1;
        let line = raw.trim();
        let at = || format!("{name}:{line_no}");
        if line_no == 1 && line.starts_with("#!") {
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let Some(body) = line.strip_prefix("//") else {
            break; // first line of code
        };
        let body = body.trim();
        if !inside && body == "==xljs==" {
            inside = true;
            continue;
        }
        if inside && body == "==/xljs==" {
            inside = false;
            done = true;
        }
        if done {
            break;
        }
        if !inside {
            continue;
        }
        let Some(c) = KEY_LINE.captures(body) else {
            continue; // free text inside the block
        };
        let key = c[1].to_lowercase();
        let val = c.get(2).map(|v| v.as_str()).unwrap_or("");
        let val = TRAILING.replace(val, "").trim().to_string();
        match key.as_str() {
            "mode" => {
                let v = val.to_lowercase();
                if v != MODE_CONSOLE && v != MODE_APP {
                    return Err(err!("meta.badMode", at(), val));
                }
                m.mode = v;
            }
            "html" => m.html = rel_path(&val, &at())?,
            "assets" => {
                for f in val.split_whitespace() {
                    m.assets.push(rel_path(f, &at())?);
                }
            }
            "title" => m.title = val,
            "size" => {
                let Some(s) = SIZE.captures(&val) else {
                    return Err(err!("meta.badSize", at(), val));
                };
                m.width = s[1].parse().unwrap_or(0);
                m.height = s[2].parse().unwrap_or(0);
            }
            "icon" => {
                let p = rel_path(&val, &at())?;
                let ext = p.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                if ext != "ico" && ext != "png" || !p.contains('.') {
                    return Err(err!("meta.badIcon", at(), val));
                }
                m.icon = p;
            }
            "titlebar" => {
                let v = val.to_lowercase();
                if v != "system" && v != "custom" {
                    return Err(err!("meta.badTitleBar", at(), val));
                }
                m.title_bar = if v == "custom" { v } else { String::new() };
            }
            "lang" => m.lang = val,
            _ => warnings.push(t!("meta.unknownKey", at(), format!("@{}", &c[1]))),
        }
    }
    if inside {
        return Err(err!("meta.unterminated", name));
    }
    Ok((m, warnings))
}

/// Cleans a slash-separated path like Go's path.Clean.
pub fn clean_path(p: &str) -> String {
    let rooted = p.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !rooted {
                    out.push("..");
                }
            }
            _ => out.push(part),
        }
    }
    let s = out.join("/");
    match (rooted, s.is_empty()) {
        (true, _) => format!("/{s}"),
        (false, true) => ".".to_string(),
        _ => s,
    }
}

/// Validates a path given in the block: relative, inside the script folder.
/// It returns the cleaned slash-separated form.
fn rel_path(p: &str, at: &str) -> Result<String> {
    if p.is_empty() {
        return Err(err!("meta.badPath", at, p));
    }
    let s = clean_path(&p.replace('\\', "/"));
    if s.starts_with('/') || s == ".." || s.starts_with("../") || s.contains(':') {
        return Err(err!("meta.badPath", at, p));
    }
    Ok(s)
}

#[cfg(test)]
mod tests;
