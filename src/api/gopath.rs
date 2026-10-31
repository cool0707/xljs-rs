//! Path functions with the semantics of Go's path/filepath (which the
//! scripts' path API was designed with): Clean, Join, Dir, Base, Ext, Rel.

#[cfg(windows)]
pub const SEP: char = '\\';
#[cfg(not(windows))]
pub const SEP: char = '/';

fn is_sep(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// The length of the volume name ("C:", "\\server\share") at the start.
#[cfg(windows)]
pub fn volume_len(p: &str) -> usize {
    let b: Vec<char> = p.chars().collect();
    if b.len() >= 2 && b[1] == ':' && b[0].is_ascii_alphabetic() {
        return 2;
    }
    if b.len() >= 5 && is_sep(b[0]) && is_sep(b[1]) && !is_sep(b[2]) && b[2] != '.' {
        // UNC: \\server\share
        let mut n = 3;
        while n < b.len() && !is_sep(b[n]) {
            n += 1;
        }
        if n < b.len() {
            n += 1;
            let start = n;
            while n < b.len() && !is_sep(b[n]) {
                n += 1;
            }
            if n > start {
                return b[..n].iter().map(|c| c.len_utf8()).sum();
            }
        }
    }
    0
}

#[cfg(not(windows))]
pub fn volume_len(_: &str) -> usize {
    0
}

pub fn is_abs(p: &str) -> bool {
    if cfg!(windows) {
        let v = volume_len(p);
        if v == 0 {
            return false;
        }
        let rest = &p[v..];
        if p.starts_with("\\\\") || p.starts_with("//") {
            return true;
        }
        rest.starts_with(is_sep)
    } else {
        p.starts_with('/')
    }
}

/// Go's filepath.Clean.
pub fn clean(path: &str) -> String {
    let vol_len = volume_len(path);
    let vol = &path[..vol_len];
    let p = &path[vol_len..];
    if p.is_empty() {
        if vol_len > 1 && !vol.ends_with(':') {
            return from_slash(vol); // UNC
        }
        return format!("{vol}.");
    }
    let rooted = p.starts_with(is_sep);
    let mut out: Vec<&str> = Vec::new();
    for part in p.split(is_sep) {
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
    let sep = SEP.to_string();
    let mut s = out.join(&sep);
    if rooted {
        s = format!("{SEP}{s}");
    }
    if s.is_empty() {
        s = ".".into();
    }
    format!("{}{s}", from_slash(vol))
}

fn from_slash(s: &str) -> String {
    if cfg!(windows) { s.replace('/', "\\") } else { s.to_string() }
}

/// Go's filepath.Join: the non-empty elements joined and cleaned.
pub fn join(parts: &[String]) -> String {
    let parts: Vec<&str> = parts.iter().map(String::as_str).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    clean(&parts.join(&SEP.to_string()))
}

/// Go's filepath.Base.
pub fn base(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let p = path.trim_end_matches(is_sep);
    let p = &p[volume_len(p).min(p.len())..];
    let b = match p.rfind(is_sep) {
        Some(i) => &p[i + 1..],
        None => p,
    };
    if b.is_empty() { SEP.to_string() } else { b.to_string() }
}

/// Go's filepath.Dir.
pub fn dir(path: &str) -> String {
    let vol = &path[..volume_len(path)];
    let rest = &path[vol.len()..];
    let i = rest.rfind(is_sep).map(|i| i + 1).unwrap_or(0);
    let d = clean(&format!("{}{}", vol, &rest[..i]));
    if d == "." && !vol.is_empty() {
        return vol.to_string();
    }
    d
}

/// Go's filepath.Ext.
pub fn ext(path: &str) -> String {
    for (i, c) in path.char_indices().rev() {
        if is_sep(c) {
            break;
        }
        if c == '.' {
            return path[i..].to_string();
        }
    }
    String::new()
}

/// Go's filepath.Abs.
pub fn abs(path: &str) -> std::io::Result<String> {
    if is_abs(path) {
        return Ok(clean(path));
    }
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    Ok(join(&[cwd, path.to_string()]))
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// Go's filepath.Rel.
pub fn rel(basepath: &str, targpath: &str) -> Result<String, String> {
    let base_vol = &basepath[..volume_len(basepath)];
    let targ_vol = &targpath[..volume_len(targpath)];
    let base = clean(basepath);
    let targ = clean(targpath);
    if same(&targ, &base) {
        return Ok(".".into());
    }
    let base = &base[base_vol.len()..];
    let targ = &targ[targ_vol.len()..];
    let base = if base == "." { "" } else { base };
    let base_slashed = base.starts_with(SEP);
    let targ_slashed = targ.starts_with(SEP);
    if base_slashed != targ_slashed || !same(base_vol, targ_vol) {
        return Err(format!("Rel: can't make {targpath} relative to {basepath}"));
    }
    let b: Vec<&str> = base.split(SEP).filter(|s| !s.is_empty()).collect();
    let t: Vec<&str> = targ.split(SEP).filter(|s| !s.is_empty()).collect();
    let common = b.iter().zip(&t).take_while(|(x, y)| same(x, y)).count();
    if b[common..].contains(&"..") {
        return Err(format!("Rel: can't make {targpath} relative to {basepath}"));
    }
    let mut out: Vec<&str> = vec![".."; b.len() - common];
    out.extend_from_slice(&t[common..]);
    if out.is_empty() {
        return Ok(".".into());
    }
    Ok(out.join(&SEP.to_string()))
}

/// Slash-separated form of a path.
pub fn to_slash(p: &str) -> String {
    if cfg!(windows) { p.replace('\\', "/") } else { p.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_semantics() {
        assert_eq!(clean("a/../b/./c/"), "b/c");
        assert_eq!(clean(""), ".");
        assert_eq!(clean("/../x"), "/x");
        assert_eq!(join(&["a".into(), "".into(), "../b".into()]), "b");
        assert_eq!(base("/a/b.txt/"), "b.txt");
        assert_eq!(base(""), ".");
        assert_eq!(dir("/a/b.txt"), "/a");
        assert_eq!(dir("b.txt"), ".");
        assert_eq!(ext("a/b.tar.gz"), ".gz");
        assert_eq!(ext("a.b/c"), "");
        assert_eq!(rel("/a/b", "/a/c/d").unwrap(), "../c/d");
        assert_eq!(rel("a", "a").unwrap(), ".");
        assert!(rel("/a", "b").is_err());
    }
}
