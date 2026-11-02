//! Simple versioning of the files the development mode replaces: before a
//! file is replaced (by an AI's answer, or by an earlier version) its
//! content is kept in <folder>/.xljs/versions/<file>/ under a number
//! (0001.js, 0002.js, ...), with a note in log.json.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::project::{DATA_DIR, read_text, write_text};
use crate::{Error, Result};

/// One kept version of a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub n: u32,
    /// The local time it was kept ("2026-11-01 15:30:00").
    pub time: String,
    /// Why (what replaced it).
    pub note: String,
}

/// The folder of the versions of the file rel (slashes) in dir.
pub fn folder(dir: &Path, rel: &str) -> PathBuf {
    let mut p = dir.join(DATA_DIR).join("versions");
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

fn file_of(dir: &Path, rel: &str, n: u32) -> PathBuf {
    let ext = Path::new(rel).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    folder(dir, rel).join(format!("{n:04}{ext}"))
}

/// The versions of a file, oldest first.
pub fn list(dir: &Path, rel: &str) -> Vec<Version> {
    let log = folder(dir, rel).join("log.json");
    let Ok(b) = std::fs::read(&log) else { return Vec::new() };
    let mut v: Vec<Version> = serde_json::from_slice(&b).unwrap_or_default();
    v.retain(|x| file_of(dir, rel, x.n).exists());
    v
}

/// The content of version n.
pub fn read(dir: &Path, rel: &str, n: u32) -> Result<String> {
    read_text(&file_of(dir, rel, n))
}

/// Keeps the current content of the file as a new version, unless the
/// file is missing or the newest version has the same content. The number
/// of the new version.
pub fn keep(dir: &Path, rel: &str, note: &str) -> Result<Option<u32>> {
    let cur = dir.join(rel);
    let Ok(text) = std::fs::read(&cur) else { return Ok(None) };
    let mut all = list(dir, rel);
    if let Some(last) = all.last()
        && std::fs::read(file_of(dir, rel, last.n)).ok().as_deref() == Some(&text[..])
    {
        return Ok(None);
    }
    let n = all.last().map(|v| v.n + 1).unwrap_or(1);
    let f = file_of(dir, rel, n);
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::path("mkdir", d.display().to_string(), e))?;
    }
    std::fs::write(&f, &text).map_err(|e| Error::path("open", f.display().to_string(), e))?;
    all.push(Version { n, time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(), note: note.to_string() });
    let log = folder(dir, rel).join("log.json");
    let json = serde_json::to_string_pretty(&all).unwrap_or_default();
    std::fs::write(&log, json).map_err(|e| Error::path("open", log.display().to_string(), e))?;
    Ok(Some(n))
}

/// Replaces the file with text, keeping its current content first.
pub fn replace(dir: &Path, rel: &str, text: &str, note: &str) -> Result<Option<u32>> {
    let kept = keep(dir, rel, note)?;
    write_text(&dir.join(rel), text)?;
    Ok(kept)
}

/// Puts version n back, keeping the current content first.
pub fn restore(dir: &Path, rel: &str, n: u32, note: &str) -> Result<String> {
    let text = read(dir, rel, n)?;
    replace(dir, rel, &text, note)?;
    Ok(text)
}
