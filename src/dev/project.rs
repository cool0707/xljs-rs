//! The script being developed: its folder, its files and where a file
//! downloaded from an AI chat belongs.

use std::path::{Path, PathBuf};

use crate::meta::Meta;
use crate::{Error, Result};

/// The extensions of the files the development mode edits.
pub const TEXT_EXTS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "json", "html", "htm", "css", "txt", "md"];

/// The extensions of scripts: a downloaded script may replace one with
/// another of these (an AI often answers .js for a .ts file).
const SCRIPT_EXTS: &[&str] = &["js", "mjs", "cjs", "ts", "mts"];

/// The folder (in the script's folder) that the development mode keeps
/// its data in: earlier versions and settings.
pub const DATA_DIR: &str = ".xljs";

/// An open script.
#[derive(Debug, Clone)]
pub struct Project {
    /// The entry script (absolute).
    pub entry: PathBuf,
    /// Its folder.
    pub dir: PathBuf,
    pub meta: Meta,
}

pub(crate) fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub(crate) fn is_script(name: &str) -> bool {
    SCRIPT_EXTS.contains(&ext_of(name).as_str())
}

fn is_text(name: &str) -> bool {
    TEXT_EXTS.contains(&ext_of(name).as_str())
}

/// The name without the extension and without the suffix that browsers
/// add to a name that is already taken ("main (1).js" → "main").
pub(crate) fn base_stem(name: &str) -> String {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let t = stem.trim_end();
    if let Some(open) = t.rfind('(')
        && t.ends_with(')')
        && t[open + 1..t.len() - 1].chars().all(|c| c.is_ascii_digit())
        && open + 2 < t.len()
    {
        return t[..open].trim_end().to_string();
    }
    t.to_string()
}

impl Project {
    /// Opens the script at path.
    pub fn open(path: &str) -> Result<Project> {
        let entry = std::path::absolute(path).map_err(|e| Error::path("open", path, e))?;
        let (meta, _) = crate::meta::parse_file(&entry.to_string_lossy())?;
        let dir = entry.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(Project { entry, dir, meta })
    }

    /// The entry script relative to the folder.
    pub fn entry_name(&self) -> String {
        self.entry.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The absolute path of a file given relative to the folder (slashes);
    /// an error for paths outside it.
    pub fn path(&self, rel: &str) -> Result<PathBuf> {
        let bad = rel.is_empty()
            || Path::new(rel).is_absolute()
            || rel.contains(':')
            || rel.split(['/', '\\']).any(|p| p == ".." || p.is_empty());
        if bad {
            return Err(crate::err!("dev.badFile", rel));
        }
        Ok(self.dir.join(rel))
    }

    /// The files that can be edited, relative to the folder with slashes:
    /// the entry script first, then the other text files of the folder
    /// and, for apps, of the asset folders.
    pub fn files(&self) -> Vec<String> {
        let entry = self.entry_name();
        let mut out = vec![entry.clone()];
        let mut add = |rel: String| {
            if !out.contains(&rel) {
                out.push(rel);
            }
        };
        let mut rest = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().map(|t| t.is_file()).unwrap_or(false) && is_text(&name) && !name.starts_with('.') && name != "jsconfig.json" && name != "xljs.d.ts" {
                    rest.push(name);
                }
            }
        }
        for root in self.meta.asset_roots() {
            let base = self.dir.join(&root);
            for e in walkdir::WalkDir::new(&base).into_iter().filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.')).flatten().take(2000) {
                if !e.file_type().is_file() {
                    continue;
                }
                let Ok(rel) = e.path().strip_prefix(&self.dir) else { continue };
                let rel = rel.to_string_lossy().replace('\\', "/");
                if is_text(&rel) && !rel.split('/').any(|p| p == "node_modules") {
                    rest.push(rel);
                }
            }
        }
        rest.sort();
        for r in rest {
            add(r);
        }
        out
    }

    /// The file a download named name replaces: the file of the same name,
    /// else the one whose name begins the download's (AI chats like to add
    /// what they changed: "集計_修正版.js" → "集計.js"; the longest wins),
    /// else for a script the file being edited (current) or the entry
    /// script. None when nothing fits (the user chooses).
    pub fn match_download(&self, name: &str, current: &str) -> Option<String> {
        let files = self.files();
        let ext = ext_of(name);
        let stem = base_stem(name).to_lowercase();
        let same_kind = |f: &str| {
            let e = ext_of(f);
            e == ext || (is_script(f) && SCRIPT_EXTS.contains(&ext.as_str()))
        };
        let file_stem = |f: &str| Path::new(f).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        // The same name (an exact extension first).
        for exact in [true, false] {
            if let Some(f) = files.iter().find(|f| file_stem(f) == stem && if exact { ext_of(f) == ext } else { same_kind(f) }) {
                return Some(f.clone());
            }
        }
        // The longest name that begins the download's.
        let best = files
            .iter()
            .filter(|f| same_kind(f))
            .filter(|f| {
                let s = file_stem(f);
                !s.is_empty() && stem.starts_with(&s)
            })
            .max_by_key(|f| (file_stem(f).chars().count(), ext_of(f) == ext));
        if let Some(f) = best {
            return Some(f.clone());
        }
        // A script for a script that is not an app: the one being edited.
        if SCRIPT_EXTS.contains(&ext.as_str()) && !self.meta.is_app() {
            if !current.is_empty() && is_script(current) && files.iter().any(|f| f == current) {
                return Some(current.to_string());
            }
            return Some(self.entry_name());
        }
        None
    }
}

/// Reads a text file: UTF-8 (without its BOM), else Shift_JIS.
pub fn read_text(p: &Path) -> Result<String> {
    let b = std::fs::read(p).map_err(|e| Error::path("open", p.display().to_string(), e))?;
    Ok(decode(&b))
}

pub fn decode(b: &[u8]) -> String {
    let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::SHIFT_JIS.decode(b).0.into_owned(),
    }
}

/// Writes a text file as UTF-8, creating its folder.
pub fn write_text(p: &Path, text: &str) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| Error::path("mkdir", d.display().to_string(), e))?;
    }
    std::fs::write(p, text).map_err(|e| Error::path("open", p.display().to_string(), e))
}
