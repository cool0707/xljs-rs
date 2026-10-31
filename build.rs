//! Generates the lists of embedded files (the built-in web pages and
//! Day.js) and, for Windows targets, the exe resources (icon, manifest).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Writes `pub static NAME: &[(&str, &[u8])]` with the files under dir.
fn embed(out: &mut String, name: &str, dir: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir);
    println!("cargo:rerun-if-changed={dir}");
    writeln!(out, "pub static {name}: &[(&str, &[u8])] = &[").unwrap();
    for f in files(&root) {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        writeln!(out, "    ({rel:?}, include_bytes!({:?})),", f.to_string_lossy()).unwrap();
    }
    writeln!(out, "];").unwrap();
}

fn main() {
    let mut s = String::new();
    embed(&mut s, "WEB_FILES", "src/ui/web");
    embed(&mut s, "DAYJS_FILES", "src/api/dayjs");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded.rs");
    std::fs::write(out, s).unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=src/cli/xljs.rc");
        println!("cargo:rerun-if-changed=src/cli/xljs.manifest");
        println!("cargo:rerun-if-changed=assets/xljs.ico");
        embed_resource::compile("src/cli/xljs.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
