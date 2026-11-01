//! Fonts for the text added to PDFs: finding a font file by its name (or
//! path), and embedding the glyphs used as a subset (a CID font with the
//! Identity-H encoding and a ToUnicode map, so the text can be searched and
//! copied).

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use subsetter::GlyphRemapper;

use crate::{Error, Result, err};

/// A font face loaded into memory.
pub struct FontFile {
    pub path: PathBuf,
    pub index: u32,
    pub data: Arc<Vec<u8>>,
    /// The PostScript name (for the PDF's BaseFont).
    pub ps_name: String,
}

impl FontFile {
    pub fn face(&self) -> ttf_parser::Face<'_> {
        // Checked when the font was loaded.
        ttf_parser::Face::parse(&self.data, self.index).expect("font parsed before")
    }
}

/// Normalizes a font name for comparing: full-width ASCII to ASCII, without
/// spaces, hyphens and underscores, lower case ("ＭＳ Ｐゴシック" = "msｐゴシック" -> "msPゴシック").
pub fn norm_name(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            _ => c,
        })
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The default fonts, tried in order for each character: the language's
/// Excel font first (游ゴシック in Japanese), then common Japanese and
/// Latin fonts of Windows, macOS and Linux.
pub fn default_names() -> Vec<String> {
    let mut v = vec![crate::i18n::current().excel.font.clone()];
    let more: &[&str] = if cfg!(windows) {
        &["Yu Gothic", "Meiryo", "MS Gothic", "Arial", "Segoe UI"]
    } else if cfg!(target_os = "macos") {
        &["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Helvetica", "Arial"]
    } else {
        &["Noto Sans CJK JP", "Noto Sans JP", "IPAexGothic", "IPAGothic", "DejaVu Sans", "Liberation Sans"]
    };
    v.extend(more.iter().map(|s| s.to_string()));
    v.retain(|s| !s.is_empty());
    v
}

/// Font files of Windows by name, so the common fonts are found without
/// reading the whole fonts folder.
const KNOWN: &[(&str, &[&str])] = &[
    ("游ゴシック", &["YuGothR.ttc", "YuGothM.ttc"]),
    ("yu gothic", &["YuGothR.ttc", "YuGothM.ttc"]),
    ("游ゴシック medium", &["YuGothM.ttc"]),
    ("yu gothic medium", &["YuGothM.ttc"]),
    ("游ゴシック light", &["YuGothL.ttc"]),
    ("yu gothic light", &["YuGothL.ttc"]),
    ("游ゴシック bold", &["YuGothB.ttc"]),
    ("yu gothic bold", &["YuGothB.ttc"]),
    ("游明朝", &["yumin.ttf"]),
    ("yu mincho", &["yumin.ttf"]),
    ("游明朝 demibold", &["yumindb.ttf"]),
    ("yu mincho demibold", &["yumindb.ttf"]),
    ("メイリオ", &["meiryo.ttc"]),
    ("meiryo", &["meiryo.ttc"]),
    ("meiryo ui", &["meiryo.ttc"]),
    ("ｍｓ ゴシック", &["msgothic.ttc"]),
    ("ms gothic", &["msgothic.ttc"]),
    ("ｍｓ ｐゴシック", &["msgothic.ttc"]),
    ("ms pgothic", &["msgothic.ttc"]),
    ("ms ui gothic", &["msgothic.ttc"]),
    ("ｍｓ 明朝", &["msmincho.ttc"]),
    ("ms mincho", &["msmincho.ttc"]),
    ("ｍｓ ｐ明朝", &["msmincho.ttc"]),
    ("ms pmincho", &["msmincho.ttc"]),
    ("biz udゴシック", &["BIZ-UDGothicR.ttc"]),
    ("biz udgothic", &["BIZ-UDGothicR.ttc"]),
    ("biz udpゴシック", &["BIZ-UDGothicR.ttc"]),
    ("biz udpgothic", &["BIZ-UDGothicR.ttc"]),
    ("biz ud明朝", &["BIZ-UDMinchoM.ttc"]),
    ("biz udmincho", &["BIZ-UDMinchoM.ttc"]),
    ("arial", &["arial.ttf"]),
    ("times new roman", &["times.ttf"]),
    ("calibri", &["calibri.ttf"]),
    ("century", &["CENTURY.TTF"]),
    ("courier new", &["cour.ttf"]),
    ("segoe ui", &["segoeui.ttf"]),
];

/// The folders with the system's (and the user's) fonts.
fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if cfg!(windows) {
        v.push(env("WINDIR").unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts"));
        if let Some(p) = env("LOCALAPPDATA") {
            v.push(p.join(r"Microsoft\Windows\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        v.push("/System/Library/Fonts".into());
        v.push("/Library/Fonts".into());
        if let Some(h) = env("HOME") {
            v.push(h.join("Library/Fonts"));
        }
    } else {
        v.push("/usr/share/fonts".into());
        v.push("/usr/local/share/fonts".into());
        if let Some(h) = env("HOME") {
            v.push(h.join(".local/share/fonts"));
            v.push(h.join(".fonts"));
        }
    }
    v
}

fn is_font_file(p: &Path) -> bool {
    let e = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(e.as_str(), "ttf" | "ttc" | "otf" | "otc")
}

fn read_at(f: &mut std::fs::File, off: u64, len: usize) -> Option<Vec<u8>> {
    if len > 64 << 20 {
        return None;
    }
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut b = vec![0; len];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The names (family, full, typographic family; every language, normalized)
/// of each face in a font file, reading only the headers and the name table.
pub fn face_names(path: &Path) -> Vec<(u32, Vec<String>)> {
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let Some(head) = read_at(&mut f, 0, 12) else { return Vec::new() };
    let offsets: Vec<u64> = if &head[..4] == b"ttcf" {
        let n = be32(&head, 8).min(256) as usize;
        match read_at(&mut f, 12, n * 4) {
            Some(b) => (0..n).map(|i| be32(&b, i * 4) as u64).collect(),
            None => return Vec::new(),
        }
    } else {
        vec![0]
    };
    let mut out = Vec::new();
    for (i, off) in offsets.into_iter().enumerate() {
        let Some(t) = read_at(&mut f, off, 12) else { continue };
        let n = u16::from_be_bytes([t[4], t[5]]) as usize;
        let Some(recs) = read_at(&mut f, off + 12, n * 16) else { continue };
        let Some(r) = (0..n).map(|k| &recs[k * 16..k * 16 + 16]).find(|r| &r[..4] == b"name") else { continue };
        let Some(data) = read_at(&mut f, be32(r, 8) as u64, be32(r, 12) as usize) else { continue };
        let Some(table) = ttf_parser::name::Table::parse(&data) else { continue };
        let mut names: Vec<String> = Vec::new();
        for name in table.names {
            if matches!(name.name_id, 1 | 4 | 16)
                && let Some(s) = name.to_string()
            {
                let s = norm_name(&s);
                if !names.contains(&s) {
                    names.push(s);
                }
            }
        }
        out.push((i as u32, names));
    }
    out
}

/// Every face of the font folders: (normalized names, path, index). It is
/// read once (only when a font is not one of the known files).
fn system_faces() -> &'static Vec<(Vec<String>, PathBuf, u32)> {
    static FACES: OnceLock<Vec<(Vec<String>, PathBuf, u32)>> = OnceLock::new();
    FACES.get_or_init(|| {
        let mut v = Vec::new();
        for dir in font_dirs() {
            for e in walkdir::WalkDir::new(&dir).follow_links(true).into_iter().flatten() {
                if e.file_type().is_file() && is_font_file(e.path()) {
                    for (i, names) in face_names(e.path()) {
                        v.push((names, e.path().to_path_buf(), i));
                    }
                }
            }
        }
        v
    })
}

/// Finds the font named name (or at a path): the face of the file whose
/// name matches, or else its first face.
pub fn locate(name: &str) -> Option<(PathBuf, u32)> {
    let p = Path::new(name);
    if is_font_file(p) || name.contains(['/', '\\']) {
        if !p.is_file() {
            return None;
        }
        let want = p.file_stem().map(|s| norm_name(&s.to_string_lossy())).unwrap_or_default();
        let idx = face_names(p).iter().find(|(_, n)| n.contains(&want)).map(|(i, _)| *i).unwrap_or(0);
        return Some((p.to_path_buf(), idx));
    }
    let want = norm_name(name);
    if want.is_empty() {
        return None;
    }
    for (k, files) in KNOWN {
        if norm_name(k) != want {
            continue;
        }
        for dir in font_dirs() {
            for f in *files {
                let path = dir.join(f);
                if path.is_file() {
                    let faces = face_names(&path);
                    if let Some((i, _)) = faces.iter().find(|(_, n)| n.contains(&want)) {
                        return Some((path, *i));
                    }
                }
            }
        }
    }
    system_faces().iter().find(|(n, _, _)| n.contains(&want)).map(|(_, p, i)| (p.clone(), *i))
}

/// Loads a face (kept for the rest of the process: fonts are big and the
/// same ones are used again).
pub fn load(path: &Path, index: u32) -> Result<Arc<FontFile>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u32), Arc<FontFile>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (path.to_path_buf(), index);
    if let Some(f) = cache.lock().unwrap().get(&key) {
        return Ok(f.clone());
    }
    let data = std::fs::read(path).map_err(|e| Error::path("open", path.to_string_lossy(), e))?;
    let face = ttf_parser::Face::parse(&data, index).map_err(|_| err!("pdfdoc.badFont", path.to_string_lossy()))?;
    // CFF2 (variable) outlines cannot be subset.
    if face.raw_face().table(ttf_parser::Tag::from_bytes(b"CFF2")).is_some() && face.tables().glyf.is_none() {
        return Err(err!("pdfdoc.badFont", path.to_string_lossy()));
    }
    let name_of = |id: u16| face.names().into_iter().filter(|n| n.name_id == id).find_map(|n| n.to_string().or_else(|| mac_roman(n.name)));
    let ps_name: String = name_of(ttf_parser::name_id::POST_SCRIPT_NAME)
        .or_else(|| name_of(ttf_parser::name_id::FAMILY))
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let ps_name = if ps_name.is_empty() { "Font".to_string() } else { ps_name };
    let f = Arc::new(FontFile { path: path.to_path_buf(), index, data: Arc::new(data), ps_name });
    cache.lock().unwrap().insert(key, f.clone());
    Ok(f)
}

/// A name of the Macintosh platform (ASCII is enough for PostScript names).
fn mac_roman(b: &[u8]) -> Option<String> {
    b.is_ascii().then(|| String::from_utf8_lossy(b).into_owned())
}

/// A font used in a document: the glyphs used so far, each with its new
/// glyph id (= the CID written in the page content), its width and its text.
pub struct FontUse {
    pub file: Arc<FontFile>,
    pub id: ObjectId,
    remapper: GlyphRemapper,
    /// By CID: the width (1/1000 em) and the text.
    glyphs: BTreeMap<u16, (f32, String)>,
    /// Embedded: further text needs a new FontUse.
    pub sealed: bool,
}

impl FontUse {
    pub fn new(file: Arc<FontFile>, id: ObjectId) -> FontUse {
        FontUse { file, id, remapper: GlyphRemapper::new(), glyphs: BTreeMap::new(), sealed: false }
    }

    /// The CID of glyph gid (for text s).
    pub fn cid(&mut self, gid: u16, s: &str) -> u16 {
        let cid = self.remapper.remap(gid);
        if !self.glyphs.contains_key(&cid) {
            let face = self.file.face();
            let w = face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as f32 * 1000.0 / face.units_per_em() as f32;
            self.glyphs.insert(cid, (w, s.to_string()));
        }
        cid
    }

    /// The width (1/1000 em) of glyphs by their CIDs.
    pub fn run_width(&self, cids: &[u16]) -> f64 {
        cids.iter().map(|c| self.glyphs.get(c).map(|g| g.0 as f64).unwrap_or(0.0)).sum()
    }

    /// Writes the font (Type0 with a CID font of the subset) into its
    /// object.
    pub fn embed(&mut self, doc: &mut Document) -> Result<()> {
        self.sealed = true;
        let fail = |e: &dyn std::fmt::Display| err!("pdfdoc.fontFailed", self.file.path.to_string_lossy(), e.to_string());
        let sub = subsetter::subset(&self.file.data, self.file.index, &self.remapper).map_err(|e| fail(&e))?;
        let face = self.file.face();
        let scale = 1000.0 / face.units_per_em() as f32;
        let tag = subset_tag(&self.glyphs);
        let base = format!("{tag}+{}", self.file.ps_name);
        let cff = face.tables().glyf.is_none();

        let font_file = if cff {
            // CFF outlines: the CFF table of the subset (FontFile3 / CIDFontType0C).
            let raw = ttf_parser::RawFace::parse(&sub, 0).map_err(|e| fail(&e))?;
            let data = raw.table(ttf_parser::Tag::from_bytes(b"CFF ")).ok_or_else(|| fail(&"no CFF table"))?.to_vec();
            let mut d = Dictionary::new();
            d.set("Subtype", Object::Name(b"CIDFontType0C".to_vec()));
            compressed(d, data)
        } else {
            let mut d = Dictionary::new();
            d.set("Length1", sub.len() as i64);
            compressed(d, sub)
        };
        let font_file_id = doc.add_object(font_file);

        let bbox = face.global_bounding_box();
        let n = |v: f32| Object::Real(v.round());
        let mut desc = Dictionary::new();
        desc.set("Type", Object::Name(b"FontDescriptor".to_vec()));
        desc.set("FontName", Object::Name(base.clone().into_bytes()));
        desc.set("Flags", 4);
        desc.set(
            "FontBBox",
            Object::Array(vec![n(bbox.x_min as f32 * scale), n(bbox.y_min as f32 * scale), n(bbox.x_max as f32 * scale), n(bbox.y_max as f32 * scale)]),
        );
        desc.set("ItalicAngle", Object::Real(face.italic_angle()));
        desc.set("Ascent", n(face.ascender() as f32 * scale));
        desc.set("Descent", n(face.descender() as f32 * scale));
        desc.set("CapHeight", n(face.capital_height().unwrap_or(face.ascender()) as f32 * scale));
        desc.set("StemV", 80);
        desc.set(if cff { "FontFile3" } else { "FontFile2" }, Object::Reference(font_file_id));
        let desc_id = doc.add_object(desc);

        let mut w = Vec::new();
        for (cid, (width, _)) in &self.glyphs {
            w.push(Object::Integer(*cid as i64));
            w.push(Object::Array(vec![Object::Real(width.round())]));
        }
        let mut sysinfo = Dictionary::new();
        sysinfo.set("Registry", Object::string_literal("Adobe"));
        sysinfo.set("Ordering", Object::string_literal("Identity"));
        sysinfo.set("Supplement", 0);
        let mut cid_font = Dictionary::new();
        cid_font.set("Type", Object::Name(b"Font".to_vec()));
        cid_font.set("Subtype", Object::Name(if cff { b"CIDFontType0".to_vec() } else { b"CIDFontType2".to_vec() }));
        cid_font.set("BaseFont", Object::Name(base.clone().into_bytes()));
        cid_font.set("CIDSystemInfo", sysinfo);
        cid_font.set("FontDescriptor", Object::Reference(desc_id));
        cid_font.set("DW", 0);
        cid_font.set("W", Object::Array(w));
        if !cff {
            cid_font.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
        }
        let cid_id = doc.add_object(cid_font);

        let to_unicode = doc.add_object(compressed(Dictionary::new(), to_unicode_cmap(&self.glyphs).into_bytes()));

        let mut font = Dictionary::new();
        font.set("Type", Object::Name(b"Font".to_vec()));
        font.set("Subtype", Object::Name(b"Type0".to_vec()));
        font.set("BaseFont", Object::Name(base.into_bytes()));
        font.set("Encoding", Object::Name(b"Identity-H".to_vec()));
        font.set("DescendantFonts", Object::Array(vec![Object::Reference(cid_id)]));
        font.set("ToUnicode", Object::Reference(to_unicode));
        doc.objects.insert(self.id, Object::Dictionary(font));
        Ok(())
    }
}

pub fn compressed(dict: Dictionary, data: Vec<u8>) -> Stream {
    let mut s = Stream::new(dict, data);
    let _ = s.compress();
    s
}

/// Six capital letters from the glyphs (the subset prefix of BaseFont).
fn subset_tag(glyphs: &BTreeMap<u16, (f32, String)>) -> String {
    let mut h: u32 = 2166136261;
    for (cid, (_, s)) in glyphs {
        for b in cid.to_be_bytes().iter().chain(s.as_bytes()) {
            h = (h ^ *b as u32).wrapping_mul(16777619);
        }
    }
    (0..6).map(|i| (b'A' + ((h >> (i * 5)) % 26) as u8) as char).collect()
}

fn utf16_hex(s: &str) -> String {
    s.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

/// The ToUnicode CMap: CID -> text.
fn to_unicode_cmap(glyphs: &BTreeMap<u16, (f32, String)>) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<_> = glyphs.iter().filter(|(_, (_, t))| !t.is_empty()).collect();
    for chunk in entries.chunks(100) {
        s.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, (_, t)) in chunk {
            s.push_str(&format!("<{cid:04X}> <{}>\n", utf16_hex(t)));
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized() {
        assert_eq!(norm_name("ＭＳ Ｐゴシック"), "msｐゴシック".replace('ｐ', "p"));
        assert_eq!(norm_name("Yu Gothic"), "yugothic");
        assert_eq!(norm_name("游ゴシック Medium"), "游ゴシックmedium");
    }

    #[test]
    fn cmap_lists_the_glyphs() {
        let mut g = BTreeMap::new();
        g.insert(1, (500.0, "A".to_string()));
        g.insert(2, (1000.0, "あ".to_string()));
        g.insert(3, (1000.0, "😀".to_string()));
        let c = to_unicode_cmap(&g);
        assert!(c.contains("3 beginbfchar\n<0001> <0041>\n<0002> <3042>\n<0003> <D83DDE00>\n"), "{c}");
        assert_eq!(subset_tag(&g).len(), 6);
    }
}
