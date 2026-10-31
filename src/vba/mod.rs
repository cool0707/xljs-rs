//! Reads the VBA source code of Excel macro workbooks: the VBA project
//! (vbaProject.bin) of .xlsm/.xlsb/.xlam/.xltm files, and the
//! _VBA_PROJECT_CUR storage of the old binary .xls/.xla files.
//!
//! The project is an OLE compound file ([MS-CFB]); its "dir" stream lists
//! the modules and every module stream holds the source compressed as
//! described in [MS-OVBA] 2.4.1, after the compiled code (MODULEOFFSET).
//! The source is stored in the project's code page (932 for Japanese Excel).

mod decompress;
mod sheets;
mod text;

use std::collections::HashMap;
use std::io::{Cursor, Read};

use crate::{Error, Result, err};

pub use decompress::decompress;
pub use text::text;

/// Module types.
pub const STANDARD: &str = "standard"; // standard module (Module1)
pub const CLASS: &str = "class"; // class module
pub const DOCUMENT: &str = "document"; // ThisWorkbook and the sheets' modules
pub const FORM: &str = "form"; // user form (its code only; the layout is not read)

/// One VBA module.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Module {
    pub name: String,
    pub kind: String,
    /// The source with LF line endings, including the Attribute lines at
    /// the top.
    pub code: String,
    /// The name of the sheet a document module belongs to, when it is known
    /// ("" for ThisWorkbook and for .xls/.xlsb files).
    pub sheet: String,
}

/// The VBA project of a workbook.
#[derive(Debug, Clone, Default)]
pub struct Project {
    pub name: String,
    pub code_page: u32,
    pub modules: Vec<Module>,
}

const CFB_SIGNATURE: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const ZIP_SIGNATURE: &[u8] = b"PK\x03\x04";

/// Reads the VBA project of the workbook at path. It returns None (and no
/// error) when the workbook has no macros.
pub fn read_file(path: &str) -> Result<Option<Project>> {
    let data = std::fs::read(path).map_err(|e| Error::path("open", path, e))?;
    read(&data).map_err(|e| e.context(path))
}

/// Reads the VBA project of a workbook (or of a vbaProject.bin) held in
/// memory. It returns None (and no error) when there are no macros.
pub fn read(data: &[u8]) -> Result<Option<Project>> {
    if data.starts_with(ZIP_SIGNATURE) {
        read_zip(data)
    } else if data.starts_with(CFB_SIGNATURE) {
        read_cfb(data, &HashMap::new())
    } else {
        Err(err!("vba.unsupported"))
    }
}

fn read_zip(data: &[u8]) -> Result<Option<Project>> {
    let mut zr = zip::ZipArchive::new(Cursor::new(data)).map_err(|_| err!("vba.unsupported"))?;
    let bin = zr.file_names().find(|n| path_base(n).eq_ignore_ascii_case("vbaProject.bin")).map(str::to_string);
    let Some(bin) = bin else {
        return Ok(None);
    };
    let mut b = Vec::new();
    zr.by_name(&bin)
        .map_err(|e| Error::msg(e.to_string()))?
        .read_to_end(&mut b)
        .map_err(|e| Error::msg(e.to_string()))?;
    let sheets = sheets::sheet_code_names(&mut zr);
    read_cfb(&b, &sheets)
}

fn path_base(name: &str) -> &str {
    match name.rfind(['/', '\\']) {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// One stream of a compound file.
struct Stream {
    path: Vec<String>, // storages, outermost first
    name: String,
    data: Vec<u8>,
}

fn corrupt(s: impl std::fmt::Display) -> Error {
    err!("vba.corrupt", s)
}

fn read_cfb(data: &[u8], sheets: &HashMap<String, String>) -> Result<Option<Project>> {
    let mut doc = cfb::CompoundFile::open(Cursor::new(data)).map_err(corrupt)?;
    let entries: Vec<(std::path::PathBuf, bool, String)> =
        doc.walk().map(|e| (e.path().to_path_buf(), e.is_stream(), e.name().to_string())).collect();
    let mut streams = Vec::new();
    for (path, is_stream, name) in entries {
        if !is_stream {
            continue;
        }
        if name == "EncryptedPackage" {
            return Err(err!("vba.encrypted"));
        }
        let storages: Vec<String> = path
            .parent()
            .map(|p| p.components().filter_map(|c| c.as_os_str().to_str()).filter(|s| *s != "/" && *s != "\\").map(str::to_string).collect())
            .unwrap_or_default();
        // Only the streams of a VBA project are needed; skip the workbook
        // itself and the forms' layouts.
        if name != "PROJECT" && !storages.last().is_some_and(|s| s.eq_ignore_ascii_case("VBA")) {
            continue;
        }
        let mut s = doc.open_stream(&path).map_err(corrupt)?;
        let mut b = Vec::new();
        s.read_to_end(&mut b).map_err(corrupt)?;
        streams.push(Stream { path: storages, name, data: b });
    }
    // The project is the storage holding VBA/dir: the root of a
    // vbaProject.bin, _VBA_PROJECT_CUR in an .xls.
    let Some(dir) = streams.iter().find(|s| s.name.eq_ignore_ascii_case("dir")) else {
        return Ok(None);
    };
    let root = &dir.path[..dir.path.len() - 1];
    let find = |path: &[String], name: &str| {
        streams.iter().find(|s| s.name.eq_ignore_ascii_case(name) && same_path(&s.path, path))
    };
    let d = decompress(&dir.data).map_err(|e| corrupt(format!("dir: {e}")))?;
    let info = parse_dir(&d).map_err(|e| corrupt(format!("dir: {e}")))?;
    let dec = |b: &[u8]| decode(info.code_page, b);
    let mut p = Project { name: dec(&info.name), code_page: info.code_page, modules: Vec::new() };

    let types = find(root, "PROJECT").map(|s| project_types(&dec(&s.data))).unwrap_or_default();
    let vba_path = dir.path.clone();
    for m in &info.modules {
        let name = if m.name_unicode.is_empty() { dec(&m.name) } else { m.name_unicode.clone() };
        let stream_name = if m.stream_unicode.is_empty() { dec(&m.stream) } else { m.stream_unicode.clone() };
        let Some(s) = find(&vba_path, &stream_name) else {
            return Err(err!("vba.moduleMissing", name));
        };
        if m.offset as usize > s.data.len() {
            return Err(corrupt(&name));
        }
        let src = decompress(&s.data[m.offset as usize..]).map_err(|e| corrupt(format!("{name}: {e}")))?;
        let mut module = Module { name: name.clone(), code: normalize_newlines(&dec(&src)), ..Default::default() };
        module.kind = lookup_fold(&types, &name);
        if module.kind.is_empty() {
            module.kind = if m.procedural { STANDARD } else { CLASS }.to_string();
        }
        if module.kind == DOCUMENT {
            module.sheet = lookup_fold(sheets, &name);
        }
        p.modules.push(module);
    }
    Ok(Some(p))
}

fn same_path(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

fn lookup_fold(m: &HashMap<String, String>, key: &str) -> String {
    if let Some(v) = m.get(key) {
        return v.clone();
    }
    let lower = key.to_lowercase();
    m.iter().find(|(k, _)| k.to_lowercase() == lower).map(|(_, v)| v.clone()).unwrap_or_default()
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Converts text in the code page to UTF-8 (Windows-1252 for unknown code
/// pages).
fn decode(cp: u32, b: &[u8]) -> String {
    if cp == 65001 {
        return String::from_utf8_lossy(b).into_owned();
    }
    let enc = match cp {
        874 => encoding_rs::WINDOWS_874,
        932 => encoding_rs::SHIFT_JIS,
        936 => encoding_rs::GBK,
        949 => encoding_rs::EUC_KR,
        950 => encoding_rs::BIG5,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        10000 => encoding_rs::MACINTOSH,
        _ => encoding_rs::WINDOWS_1252,
    };
    let (s, _, bad) = enc.decode(b);
    if bad {
        return String::from_utf8_lossy(b).into_owned();
    }
    s.into_owned()
}

/// What the "dir" stream says about the project.
struct DirInfo {
    name: Vec<u8>,
    code_page: u32,
    modules: Vec<DirModule>,
}

#[derive(Default, Clone)]
struct DirModule {
    name: Vec<u8>,
    stream: Vec<u8>,
    name_unicode: String,
    stream_unicode: String,
    offset: u32,
    procedural: bool,
}

/// Reads the records of the decompressed "dir" stream ([MS-OVBA]
/// 2.3.4.2). Every record is an id, a size and that many bytes, except
/// PROJECTVERSION whose size field (always 4) is followed by 6 bytes.
fn parse_dir(d: &[u8]) -> Result<DirInfo, String> {
    let mut info = DirInfo { name: Vec::new(), code_page: 1252, modules: Vec::new() };
    let mut cur: Option<DirModule> = None;
    let mut pos = 0;
    while pos + 6 <= d.len() {
        let id = u16::from_le_bytes([d[pos], d[pos + 1]]);
        let mut size = u32::from_le_bytes(d[pos + 2..pos + 6].try_into().unwrap()) as usize;
        pos += 6;
        if id == 0x0009 {
            size = 6; // PROJECTVERSION
        }
        if pos + size > d.len() {
            return Err("truncated record".into());
        }
        let v = &d[pos..pos + size];
        pos += size;
        match id {
            0x0003 if v.len() >= 2 => info.code_page = u16::from_le_bytes([v[0], v[1]]) as u32,
            0x0004 => info.name = v.to_vec(),
            0x0019 => {
                // MODULENAME starts a module
                if let Some(m) = cur.take() {
                    info.modules.push(m);
                }
                cur = Some(DirModule { name: v.to_vec(), ..Default::default() });
            }
            0x0047 => {
                if let Some(m) = cur.as_mut() {
                    m.name_unicode = utf16le(v);
                }
            }
            0x001A => {
                if let Some(m) = cur.as_mut() {
                    m.stream = v.to_vec();
                }
            }
            0x0032 => {
                if let Some(m) = cur.as_mut() {
                    m.stream_unicode = utf16le(v);
                }
            }
            0x0031 => {
                if let Some(m) = cur.as_mut()
                    && v.len() >= 4
                {
                    m.offset = u32::from_le_bytes(v[..4].try_into().unwrap());
                }
            }
            0x0021 => {
                if let Some(m) = cur.as_mut() {
                    m.procedural = true;
                }
            }
            0x002B => {
                if let Some(m) = cur.take() {
                    info.modules.push(m);
                }
            }
            0x0010 => pos = d.len(),
            _ => {}
        }
    }
    if let Some(m) = cur {
        info.modules.push(m);
    }
    Ok(info)
}

fn utf16le(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&u)
}

/// Reads the module types from the PROJECT stream ([MS-OVBA] 2.3.1):
/// Module=, Document=name/&H..., Class= and BaseClass= (forms).
fn project_types(text: &str) -> HashMap<String, String> {
    let mut types = HashMap::new();
    for line in normalize_newlines(text).split('\n') {
        if line.starts_with('[') {
            break;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (t, value) = match key.trim() {
            "Module" => (STANDARD, value),
            "Class" => (CLASS, value),
            "BaseClass" => (FORM, value),
            "Document" => (DOCUMENT, value.split('/').next().unwrap_or("")),
            _ => continue,
        };
        types.insert(value.trim().to_string(), t.to_string());
    }
    types
}

#[cfg(test)]
mod tests;
