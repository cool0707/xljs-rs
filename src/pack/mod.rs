//! Embeds a bundled script into a copy of the xljs executable and reads it
//! back at startup.
//!
//! Layout appended to the executable (v2):
//!
//! ```text
//! [zip archive][payload length u64 LE][magic "XLJSPAK2"]
//! ```
//!
//! The zip holds manifest.json (script name and metadata), main.js (the
//! bundled code) and assets/... (files listed by @html and @assets).
//! Executables made by older versions (v1) are still read:
//!
//! ```text
//! [name length u32][name][code][payload length u64][magic "XLJSPAK1"]
//! ```

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::meta::{AssetFs, MemAssets, Meta};
use crate::{Error, Result, err};

const MAGIC_V1: &[u8; 8] = b"XLJSPAK1";
const MAGIC_V2: &[u8; 8] = b"XLJSPAK2";
const TRAILER_SIZE: u64 = 16;

const MANIFEST_NAME: &str = "manifest.json";
const CODE_NAME: &str = "main.js";
const ASSETS_DIR: &str = "assets";

/// An embedded script.
pub struct Payload {
    pub name: String,
    pub code: Vec<u8>,
    /// The script's metadata block (None for v1 payloads).
    pub meta: Option<Meta>,
    /// The files listed by Meta::asset_roots. When writing, they are read
    /// from this file system (the script folder); when reading, it is the
    /// embedded copy. None when there are none.
    pub assets: Option<Box<dyn AssetFs>>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: i32,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meta: Option<Meta>,
}

fn corrupt() -> Error {
    Error::msg("corrupt payload")
}

/// The payload of the running executable, or None when it has none.
pub fn read_self() -> Result<Option<Payload>> {
    let exe = std::env::current_exe()?;
    read(&exe)
}

/// The payload appended to the file at path, or None.
pub fn read(file: &Path) -> Result<Option<Payload>> {
    let mut f = File::open(file).map_err(|e| Error::path("open", file.display().to_string(), e))?;
    let Some((version, buf)) = read_payload(&mut f)? else {
        return Ok(None);
    };
    if version == 1 {
        if buf.len() < 4 {
            return Err(corrupt());
        }
        let n = u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize;
        if 4 + n > buf.len() {
            return Err(corrupt());
        }
        let name = String::from_utf8_lossy(&buf[4..4 + n]).into_owned();
        return Ok(Some(Payload { name, code: buf[4 + n..].to_vec(), meta: None, assets: None }));
    }
    let mut zr = zip::ZipArchive::new(io::Cursor::new(buf)).map_err(|_| corrupt())?;
    let read_entry = |zr: &mut zip::ZipArchive<io::Cursor<Vec<u8>>>, name: &str| -> Result<Vec<u8>> {
        let mut e = zr.by_name(name).map_err(|_| corrupt())?;
        let mut data = Vec::new();
        e.read_to_end(&mut data).map_err(|_| corrupt())?;
        Ok(data)
    };
    let manifest: Manifest = serde_json::from_slice(&read_entry(&mut zr, MANIFEST_NAME)?).map_err(|_| corrupt())?;
    let code = read_entry(&mut zr, CODE_NAME)?;
    let mut assets = MemAssets::new();
    let names: Vec<String> = zr.file_names().map(str::to_string).collect();
    for n in names {
        if let Some(rest) = n.strip_prefix(&format!("{ASSETS_DIR}/"))
            && !n.ends_with('/')
        {
            let data = read_entry(&mut zr, &n)?;
            assets.insert(rest, data);
        }
    }
    let assets: Option<Box<dyn AssetFs>> = if assets.files.is_empty() { None } else { Some(Box::new(assets)) };
    Ok(Some(Payload { name: manifest.name, code, meta: manifest.meta, assets }))
}

/// The payload version and bytes, or None when the file has none.
fn read_payload(f: &mut File) -> Result<Option<(u32, Vec<u8>)>> {
    let Some((size, version)) = payload_size(f)? else {
        return Ok(None);
    };
    let total = f.metadata()?.len();
    f.seek(SeekFrom::Start(total - TRAILER_SIZE - size))?;
    let mut buf = vec![0u8; size as usize];
    f.read_exact(&mut buf)?;
    Ok(Some((version, buf)))
}

/// Reads the trailer: the payload size and version, None without payload.
fn payload_size(f: &mut File) -> Result<Option<(u64, u32)>> {
    let total = f.metadata()?.len();
    if total < TRAILER_SIZE {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(total - TRAILER_SIZE))?;
    let mut trailer = [0u8; TRAILER_SIZE as usize];
    f.read_exact(&mut trailer)?;
    let version = match &trailer[8..] {
        m if m == MAGIC_V1 => 1,
        m if m == MAGIC_V2 => 2,
        _ => return Ok(None),
    };
    let size = u64::from_le_bytes(trailer[..8].try_into().unwrap());
    if size < 4 || size > total - TRAILER_SIZE {
        return Err(corrupt());
    }
    Ok(Some((size, version)))
}

/// Copies the running executable (without any existing payload) to out
/// and appends the script.
pub fn write(out: &Path, p: &Payload) -> Result<()> {
    let exe = std::env::current_exe()?;
    write_from(&exe, out, p)
}

/// Copies base (stripping an existing payload) to out and appends the
/// script.
pub fn write_from(base: &Path, out: &Path, p: &Payload) -> Result<()> {
    if same_file(base, out) {
        return Err(err!("pack.sameAsSelf", out.display()));
    }
    let mut archive = build_archive(p)?;
    let mut src = File::open(base).map_err(|e| Error::path("open", base.display().to_string(), e))?;
    let mut size = src.metadata()?.len();
    if let Ok(Some((old, _))) = payload_size(&mut src) {
        size -= old + TRAILER_SIZE;
    }
    src.seek(SeekFrom::Start(0))?;
    let mut dst = File::create(out).map_err(|e| Error::path("open", out.display().to_string(), e))?;
    io::copy(&mut (&mut src).take(size), &mut dst)?;
    let n = archive.len() as u64;
    archive.extend_from_slice(&n.to_le_bytes());
    archive.extend_from_slice(MAGIC_V2);
    dst.write_all(&archive)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

fn build_archive(p: &Payload) -> Result<Vec<u8>> {
    let mut zw = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let zerr = |e: zip::result::ZipError| Error::msg(e.to_string());
    let manifest = serde_json::to_vec(&Manifest { version: 2, name: p.name.clone(), meta: p.meta.clone() })?;
    zw.start_file(MANIFEST_NAME, opts).map_err(zerr)?;
    zw.write_all(&manifest)?;
    zw.start_file(CODE_NAME, opts).map_err(zerr)?;
    zw.write_all(&p.code)?;
    if let (Some(assets), Some(meta)) = (&p.assets, &p.meta) {
        let mut seen = std::collections::HashSet::new();
        for root in meta.asset_roots() {
            let files = assets.files(&root).map_err(|e| Error::path("open", root.clone(), e))?;
            for name in files {
                if !seen.insert(name.clone()) {
                    continue;
                }
                let data = assets.read(&name).map_err(|e| Error::path("open", name.clone(), e))?;
                zw.start_file(format!("{ASSETS_DIR}/{name}"), opts).map_err(zerr)?;
                zw.write_all(&data)?;
            }
        }
    }
    Ok(zw.finish().map_err(zerr)?.into_inner())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
