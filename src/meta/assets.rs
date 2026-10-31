use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// A read-only tree of files addressed by slash-separated relative paths
/// (like Go's io/fs): the files served to windows and packed into exes.
pub trait AssetFs: Send + Sync {
    /// The contents of the file name.
    fn read(&self, name: &str) -> io::Result<Vec<u8>>;
    /// The files at or under root (a file or a folder; "." for all), sorted.
    fn files(&self, root: &str) -> io::Result<Vec<String>>;
    /// Whether name is a file.
    fn is_file(&self, name: &str) -> bool {
        self.read(name).is_ok()
    }
}

fn not_found(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("open {name}: file does not exist"))
}

/// Reports whether name is a valid path for an [`AssetFs`]: unrooted,
/// slash-separated, without empty, "." or ".." elements ("." alone is the
/// root).
pub fn valid_path(name: &str) -> bool {
    if name == "." {
        return true;
    }
    !name.is_empty() && name.split('/').all(|e| !e.is_empty() && e != "." && e != ".." && !e.contains('\\'))
}

/// Whether name is one of roots or inside one of them.
pub fn covered(roots: &[String], name: &str) -> bool {
    roots
        .iter()
        .any(|r| r == "." || name == r || name.strip_prefix(r.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

/// Files of a folder on disk, limited to some roots (the script's assets).
#[derive(Debug, Clone)]
pub struct DirAssets {
    base: PathBuf,
    roots: Vec<String>,
}

impl DirAssets {
    pub fn new(base: &Path, roots: Vec<String>) -> Self {
        DirAssets { base: base.to_path_buf(), roots }
    }
}

impl AssetFs for DirAssets {
    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        if !valid_path(name) || !covered(&self.roots, name) {
            return Err(not_found(name));
        }
        let p = self.base.join(name);
        if p.is_dir() {
            return Err(io::Error::new(io::ErrorKind::IsADirectory, format!("{name} is a directory")));
        }
        std::fs::read(p)
    }

    fn files(&self, root: &str) -> io::Result<Vec<String>> {
        if !valid_path(root) || !covered(&self.roots, root) {
            return Err(not_found(root));
        }
        let start = if root == "." { self.base.clone() } else { self.base.join(root) };
        let meta = std::fs::metadata(&start)?;
        if meta.is_file() {
            return Ok(vec![root.to_string()]);
        }
        let mut out = Vec::new();
        for e in walkdir::WalkDir::new(&start).sort_by_file_name() {
            let e = e.map_err(io::Error::other)?;
            if !e.file_type().is_file() {
                continue;
            }
            let rel = e.path().strip_prefix(&self.base).unwrap_or(e.path());
            let rel: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            let name = rel.join("/");
            if covered(&self.roots, &name) {
                out.push(name);
            }
        }
        Ok(out)
    }
}

/// Files held in memory (packed exes, the debugger's files).
#[derive(Debug, Clone, Default)]
pub struct MemAssets {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl MemAssets {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, name: impl Into<String>, data: impl Into<Vec<u8>>) {
        self.files.insert(name.into(), data.into());
    }
}

impl AssetFs for MemAssets {
    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        self.files.get(name).cloned().ok_or_else(|| not_found(name))
    }

    fn files(&self, root: &str) -> io::Result<Vec<String>> {
        let roots = [root.to_string()];
        let out: Vec<String> = self.files.keys().filter(|k| covered(&roots, k)).cloned().collect();
        if out.is_empty() {
            return Err(not_found(root));
        }
        Ok(out)
    }

    fn is_file(&self, name: &str) -> bool {
        self.files.contains_key(name)
    }
}
