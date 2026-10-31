use std::io::Write;
use std::path::Path;

use globset::GlobBuilder;
use rquickjs::Value;

use super::gopath;
use super::js::{self, object, v};
use super::{N, R};
use crate::{Error, err};

fn io_err(op: &str, path: &str, e: std::io::Error) -> Error {
    Error::path(op, path, e)
}

pub(super) fn install(n: &N) {
    n.set("fsStat", |_, a| {
        let p = a.str(0);
        match std::fs::metadata(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => a.null(),
            Err(e) => Err(a.throw(&io_err("stat", &p, e))),
            Ok(st) => {
                let mtime = st.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as f64).unwrap_or(0.0);
                let c = &a.ctx;
                object(
                    c,
                    vec![("isFile", v(c, st.is_file())), ("isDirectory", v(c, st.is_dir())), ("size", v(c, st.len() as f64)), ("mtimeMs", v(c, mtime))],
                )
            }
        }
    });
    n.set("fsReadBytes", |_, a| {
        let p = a.str(0);
        let b = std::fs::read(&p).map_err(|e| a.throw(&io_err("open", &p, e)))?;
        js::array_buffer(&a.ctx, b)
    });
    n.set("fsReadText", |_, a| {
        let p = a.str(0);
        let b = std::fs::read(&p).map_err(|e| a.throw(&io_err("open", &p, e)))?;
        let s = super::decode(&b, &a.opt_str(1, "auto")).map_err(|e| a.throw(&e))?;
        a.ret(s)
    });
    n.set("fsWriteBytes", |_, a| {
        let data = a.bytes(1)?;
        write_file(&a.str(0), &data, a.bool(2)).map_err(|e| a.throw(&e))?;
        a.undefined()
    });
    n.set("fsWriteText", |_, a| {
        let path = a.str(0);
        let append = a.bool(3);
        let mut enc = a.opt_str(2, "utf8");
        if append && super::encoding::is_utf8_bom(&enc) {
            // Only write the BOM when creating the file.
            if std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
                enc = "utf8".into();
            }
        }
        let b = super::encode(&a.str(1), &enc).map_err(|e| a.throw(&e))?;
        write_file(&path, &b, append).map_err(|e| a.throw(&e))?;
        a.undefined()
    });
    n.set("fsReadDir", |_, a| {
        let p = a.str(0);
        let hidden = a.bool(1);
        let rd = std::fs::read_dir(&p).map_err(|e| a.throw(&io_err("readdir", &p, e)))?;
        let mut entries: Vec<(String, bool)> = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ft = e.file_type().ok();
            let is_dir = ft.map(|t| t.is_dir()).unwrap_or(false)
                || (ft.is_some_and(|t| t.is_symlink()) && e.path().is_dir());
            if !hidden && is_hidden(&name, &e.path()) {
                continue;
            }
            entries.push((name, is_dir));
        }
        entries.sort();
        let c = &a.ctx;
        let vals: Vec<Value> = entries
            .into_iter()
            .map(|(name, d)| object(c, vec![("name", v(c, name)), ("isDirectory", v(c, d))]).unwrap())
            .collect();
        js::array(c, vals)
    });
    n.set("fsWalk", |_, a| {
        let res = walk(&a.str(0), &a.strings(1), &a.strings(2), a.bool(3), !a.bool(4), a.bool(5)).map_err(|e| a.throw(&e))?;
        walk_result(a, res)
    });
    n.set("fsGlob", |_, a| {
        let pattern = gopath::to_slash(&a.str(1));
        let res = walk(&a.str(0), &[pattern], &[], false, true, false).map_err(|e| a.throw(&e))?;
        walk_result(a, res)
    });
    n.set("fsMkdir", |_, a| {
        let p = a.str(0);
        std::fs::create_dir_all(&p).map_err(|e| a.throw(&io_err("mkdir", &p, e)))?;
        a.undefined()
    });
    n.set("fsRemove", |_, a| {
        let p = a.str(0);
        let r = if a.bool(1) {
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(&p),
                Ok(_) => std::fs::remove_file(&p),
                Err(e) => Err(e),
            }
        } else {
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => std::fs::remove_dir(&p),
                _ => std::fs::remove_file(&p),
            }
        };
        match r {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(a.throw(&io_err("remove", &p, e))),
            _ => a.undefined(),
        }
    });
    n.set("fsRename", |_, a| {
        let (from, to) = (a.str(0), a.str(1));
        std::fs::rename(&from, &to)
            .map_err(|e| a.throw(&Error::Link { op: "rename".into(), old: from.clone(), new: to.clone(), source: e }))?;
        a.undefined()
    });
    n.set("fsCopy", |_, a| {
        copy_file(&a.str(0), &a.str(1)).map_err(|e| a.throw(&e))?;
        a.undefined()
    });
    n.set("fsCwd", |_, a| {
        let d = std::env::current_dir().map_err(|e| a.throw(&e.into()))?;
        a.ret(d.to_string_lossy().into_owned())
    });
    n.set("fsChdir", |_, a| {
        let p = a.str(0);
        std::env::set_current_dir(&p).map_err(|e| a.throw(&io_err("chdir", &p, e)))?;
        a.undefined()
    });
    n.set("knownFolder", |_, a| match known_folder(&a.str(0)) {
        Some(p) => a.ret(p),
        None => a.null(),
    });

    // path
    n.set("pathJoin", |_, a| {
        let parts: Vec<String> = a.v.iter().map(js::to_str).collect();
        a.ret(gopath::join(&parts))
    });
    n.set("pathResolve", |_, a| {
        let mut p = String::new();
        for x in &a.v {
            let s = js::to_str(x);
            if gopath::is_abs(&s) || p.is_empty() {
                p = s;
            } else {
                p = gopath::join(&[p, s]);
            }
        }
        let abs = gopath::abs(&p).map_err(|e| a.throw(&e.into()))?;
        a.ret(abs)
    });
    n.set("pathDirname", |_, a| a.ret(gopath::dir(&a.str(0))));
    n.set("pathBasename", |_, a| {
        let mut b = gopath::base(&a.str(0));
        let ext = a.str(1);
        if !ext.is_empty() && b.ends_with(&ext) && b != ext {
            b.truncate(b.len() - ext.len());
        }
        a.ret(b)
    });
    n.set("pathExtname", |_, a| a.ret(gopath::ext(&a.str(0))));
    n.set("pathNormalize", |_, a| a.ret(gopath::clean(&a.str(0))));
    n.set("pathIsAbsolute", |_, a| a.ret(gopath::is_abs(&a.str(0))));
    n.set("pathRelative", |_, a| {
        let r = gopath::rel(&a.str(0), &a.str(1)).map_err(|e| a.throw(&Error::msg(e)))?;
        a.ret(r)
    });
    n.val("pathSep", gopath::SEP.to_string());
}

struct WalkEntry {
    path: String,
    rel: String,
    dir: bool,
}

fn walk_result<'js>(a: &super::A<'js>, res: Vec<WalkEntry>) -> R<'js> {
    let c = &a.ctx;
    let vals: Vec<Value> = res
        .into_iter()
        .map(|e| object(c, vec![("path", v(c, e.path)), ("rel", v(c, e.rel)), ("isDirectory", v(c, e.dir))]).unwrap())
        .collect();
    js::array(c, vals)
}

fn write_file(path: &str, data: &[u8], append: bool) -> crate::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true);
    if append {
        o.append(true);
    } else {
        o.truncate(true);
    }
    let mut f = o.open(path).map_err(|e| io_err("open", path, e))?;
    f.write_all(data).map_err(|e| io_err("write", path, e))?;
    f.flush().map_err(|e| io_err("close", path, e))?;
    Ok(())
}

fn copy_file(src: &str, dst: &str) -> crate::Result<()> {
    let st = std::fs::metadata(src).map_err(|e| io_err("open", src, e))?;
    if st.is_dir() {
        return Err(err!("fs.copyDirectory", src));
    }
    std::fs::copy(src, dst).map_err(|e| io_err("open", dst, e))?;
    if let Ok(m) = st.modified() {
        let f = std::fs::File::options().write(true).open(dst).map_err(|e| io_err("open", dst, e))?;
        f.set_modified(m).map_err(|e| io_err("chtimes", dst, e))?;
    }
    Ok(())
}

/// Matches a slash-separated relative path against a glob. Patterns
/// without a slash match the base name at any depth. Matching is
/// case-insensitive (Windows semantics).
pub(crate) fn match_pattern(pattern: &str, rel: &str) -> bool {
    let pattern = gopath::to_slash(pattern).to_lowercase();
    let rel = rel.to_lowercase();
    let build = |p: &str| GlobBuilder::new(p).literal_separator(true).backslash_escape(true).build().map(|g| g.compile_matcher());
    if !pattern.contains('/') {
        let base = rel.rsplit('/').next().unwrap_or(&rel);
        return build(&pattern).map(|m| m.is_match(base)).unwrap_or(false);
    }
    let pattern = pattern.strip_prefix("./").unwrap_or(&pattern);
    build(pattern).map(|m| m.is_match(&rel)).unwrap_or(false)
}

fn match_any(patterns: &[String], rel: &str) -> bool {
    patterns.iter().any(|p| match_pattern(p, rel))
}

/// Whether a directory entry is left out of listings by default: hidden or
/// system files (on Windows), names starting with "." and Office lock
/// files ("~$Book1.xlsx").
fn is_hidden(name: &str, path: &Path) -> bool {
    name.starts_with('.') || name.starts_with("~$") || has_hidden_attribute(path)
}

#[cfg(windows)]
fn has_hidden_attribute(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    std::fs::symlink_metadata(path).map(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0).unwrap_or(false)
}

#[cfg(not(windows))]
fn has_hidden_attribute(_: &Path) -> bool {
    false
}

/// Lists the entries under root. Hidden entries and the contents of hidden
/// directories are skipped unless hidden is set.
fn walk(root: &str, include: &[String], exclude: &[String], dirs: bool, files: bool, hidden: bool) -> crate::Result<Vec<WalkEntry>> {
    let abs = gopath::abs(root).map_err(|e| io_err("stat", root, e))?;
    let st = std::fs::metadata(&abs).map_err(|e| io_err("stat", root, e))?;
    if !st.is_dir() {
        return Err(err!("fs.notDirectory", root));
    }
    let mut out = Vec::new();
    let mut it = walkdir::WalkDir::new(&abs).sort_by_file_name().min_depth(1).into_iter();
    while let Some(e) = it.next() {
        let Ok(e) = e else { continue }; // unreadable entries are skipped
        let is_dir = e.file_type().is_dir();
        let name = e.file_name().to_string_lossy().into_owned();
        if !hidden && is_hidden(&name, e.path()) {
            if is_dir {
                it.skip_current_dir();
            }
            continue;
        }
        let p = e.path().to_string_lossy().into_owned();
        let rel = gopath::rel(&abs, &p).unwrap_or_else(|_| name.clone());
        let slash = gopath::to_slash(&rel);
        if !exclude.is_empty() && match_any(exclude, &slash) {
            if is_dir {
                it.skip_current_dir();
            }
            continue;
        }
        if (is_dir && !dirs) || (!is_dir && !files) {
            continue;
        }
        if !include.is_empty() && !match_any(include, &slash) {
            continue;
        }
        out.push(WalkEntry { path: p, rel, dir: is_dir });
    }
    Ok(out)
}

#[cfg(windows)]
fn known_folder(name: &str) -> Option<String> {
    use windows::Win32::UI::Shell::*;
    let id = match name.to_lowercase().as_str() {
        "desktop" => &FOLDERID_Desktop,
        "documents" => &FOLDERID_Documents,
        "downloads" => &FOLDERID_Downloads,
        "music" => &FOLDERID_Music,
        "pictures" => &FOLDERID_Pictures,
        "videos" => &FOLDERID_Videos,
        _ => return None,
    };
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

#[cfg(not(windows))]
fn known_folder(name: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let sub = match name.to_lowercase().as_str() {
        "desktop" => "Desktop",
        "documents" => "Documents",
        "downloads" => "Downloads",
        "music" => "Music",
        "pictures" => "Pictures",
        "videos" => "Videos",
        _ => return None,
    };
    Some(Path::new(&home).join(sub).to_string_lossy().into_owned())
}
