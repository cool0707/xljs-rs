//! Editing PDF files (the global `pdf`; natives in src/api/pdf.rs): joining
//! documents, removing, inserting and rotating pages, and adding text. The
//! file is read with lopdf, and the page tree is kept flat (one Pages node
//! whose kids are the pages in order, with the inherited attributes put on
//! each page) so pages move freely between documents.

pub mod font;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId, Stream};
use serde::Deserialize;

use crate::{Error, Result, err};
use font::{FontFile, FontUse};

/// The attributes a page inherits from the Pages nodes above it.
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

const MM: f64 = 72.0 / 25.4;

/// An open PDF document.
pub struct PdfDoc {
    pub doc: Document,
    /// The file it was opened from or last saved to ("" for a new one).
    pub path: String,
    /// The pages in order.
    pub pages: Vec<ObjectId>,
    root: ObjectId,
    fonts: Vec<FontUse>,
    /// Pages whose content was put in q ... Q before text was added.
    wrapped: HashSet<ObjectId>,
    /// Pages were removed, inserted or moved (the page labels no longer fit).
    restructured: bool,
    pub closed: bool,
}

fn lerr(e: lopdf::Error) -> Error {
    err!("pdfdoc.invalid", e.to_string())
}

fn num(o: &Object, doc: &Document) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| num(o, doc)),
        _ => None,
    }
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(o),
        _ => o,
    }
}

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

/// A number for page content (at most 3 decimals).
fn f(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The attributes page inherits, from its parents (nearest first).
fn inherited(doc: &Document, page: ObjectId) -> Vec<(&'static [u8], Object)> {
    let mut out = Vec::new();
    let Ok(d) = doc.get_dictionary(page) else { return out };
    let mut cur = d.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
    let mut seen = HashSet::new();
    while let Some(id) = cur {
        if !seen.insert(id) || seen.len() > 64 {
            break;
        }
        let Ok(node) = doc.get_dictionary(id) else { break };
        for k in INHERITED {
            if !d.has(k) && !out.iter().any(|(x, _)| *x == k)
                && let Ok(v) = node.get(k)
            {
                out.push((k, v.clone()));
            }
        }
        cur = node.get(b"Parent").ok().and_then(|p| p.as_reference().ok());
    }
    out
}

/// Text alignment of addText.
#[derive(Deserialize, Default, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// The options of addText (lengths in mm, the size in pt).
#[derive(Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct TextOptions {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub right: Option<f64>,
    pub bottom: Option<f64>,
    pub size: f64,
    pub color: String,
    pub opacity: f64,
    pub align: Align,
    pub line_height: f64,
    pub rotate: f64,
    /// The fonts to use, by name or path, tried in order for each character
    /// (empty: font::default_names).
    pub font: Vec<String>,
}

impl Default for TextOptions {
    fn default() -> Self {
        TextOptions {
            x: None,
            y: None,
            right: None,
            bottom: None,
            size: 11.0,
            color: String::new(),
            opacity: 1.0,
            align: Align::Left,
            line_height: 1.3,
            rotate: 0.0,
            font: Vec::new(),
        }
    }
}

/// "#rgb", "#rrggbb" or a basic color name, as 0..1 components.
pub fn parse_color(s: &str) -> Option<[f64; 3]> {
    let s = s.trim().to_ascii_lowercase();
    let hex = |h: &str| -> Option<[f64; 3]> {
        let v: Vec<u8> = match h.len() {
            3 => h.chars().map(|c| c.to_digit(16).map(|d| (d * 17) as u8)).collect::<Option<_>>()?,
            6 => (0..3).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()).collect::<Option<_>>()?,
            _ => return None,
        };
        Some([v[0] as f64 / 255.0, v[1] as f64 / 255.0, v[2] as f64 / 255.0])
    };
    if let Some(h) = s.strip_prefix('#') {
        return hex(h);
    }
    let h = match s.as_str() {
        "" | "black" => "000000",
        "white" => "ffffff",
        "red" => "ff0000",
        "green" => "008000",
        "blue" => "0000ff",
        "yellow" => "ffff00",
        "orange" => "ffa500",
        "purple" => "800080",
        "gray" | "grey" => "808080",
        "silver" => "c0c0c0",
        "navy" => "000080",
        _ => return None,
    };
    hex(h)
}

/// The size (mm) and rotation of a page as it is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageInfo {
    pub width: f64,
    pub height: f64,
    pub rotation: i64,
}

/// The fonts of addText, tried in order for each character.
struct FontList {
    names: Vec<String>,
    /// By name: None = not looked for yet, Some(None) = not found.
    files: Vec<Option<Option<Arc<FontFile>>>>,
}

impl FontList {
    fn get(&mut self, k: usize) -> Option<Arc<FontFile>> {
        if self.files.len() <= k {
            self.files.resize(k + 1, None);
        }
        self.files[k].get_or_insert_with(|| resolve_font(&self.names[k])).clone()
    }

    /// The first font that exists (fonts_for checked there is one).
    fn first(&mut self) -> Arc<FontFile> {
        (0..self.names.len()).find_map(|k| self.get(k)).expect("a font")
    }

    /// The font with a glyph for c (or else the first font's .notdef), the
    /// glyph and its advance (em).
    fn glyph(&mut self, c: char) -> (Arc<FontFile>, u16, f64) {
        let found = (0..self.names.len()).find_map(|k| {
            let f = self.get(k)?;
            let g = f.face().glyph_index(c)?;
            Some((f, g.0))
        });
        let (f, gid) = found.unwrap_or_else(|| (self.first(), 0));
        let face = f.face();
        let adv = face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as f64 / face.units_per_em() as f64;
        (f, gid, adv)
    }
}

/// A font face found by name (or path), remembered for the process.
fn resolve_font(name: &str) -> Option<Arc<FontFile>> {
    static FOUND: OnceLock<Mutex<HashMap<String, Option<(PathBuf, u32)>>>> = OnceLock::new();
    let found = FOUND.get_or_init(Default::default);
    let hit = found.lock().unwrap().get(name).cloned();
    let loc = match hit {
        Some(l) => l,
        None => {
            let l = font::locate(name);
            found.lock().unwrap().insert(name.to_string(), l.clone());
            l
        }
    };
    let (p, i) = loc?;
    font::load(&p, i).ok()
}

impl PdfDoc {
    /// A new document without pages.
    pub fn create() -> PdfDoc {
        let mut doc = Document::with_version("1.7");
        let root = doc.new_object_id();
        let mut pages = Dictionary::new();
        pages.set("Type", name("Pages"));
        pages.set("Kids", Object::Array(Vec::new()));
        pages.set("Count", 0);
        doc.objects.insert(root, Object::Dictionary(pages));
        let mut cat = Dictionary::new();
        cat.set("Type", name("Catalog"));
        cat.set("Pages", Object::Reference(root));
        let cat_id = doc.add_object(cat);
        doc.trailer.set("Root", Object::Reference(cat_id));
        PdfDoc::with(doc, root, Vec::new(), String::new())
    }

    fn with(doc: Document, root: ObjectId, pages: Vec<ObjectId>, path: String) -> PdfDoc {
        PdfDoc { doc, path, pages, root, fonts: Vec::new(), wrapped: HashSet::new(), restructured: false, closed: false }
    }

    /// Reads a PDF file's bytes (password: of an encrypted file; the copy
    /// in memory, and so the saved file, is not encrypted).
    pub fn open(data: &[u8], password: &str, path: String) -> Result<PdfDoc> {
        if !data.starts_with(b"%PDF") && !data[..data.len().min(1024)].windows(4).any(|w| w == b"%PDF") {
            return Err(err!("pdfdoc.notPdf"));
        }
        let opts = LoadOptions { password: (!password.is_empty()).then(|| password.to_string()), ..Default::default() };
        let mut doc = Document::load_mem_with_options(data, opts).map_err(|e| match e {
            lopdf::Error::InvalidPassword => err!("pdfdoc.badPassword"),
            e => lerr(e),
        })?;
        if doc.trailer.has(b"Encrypt") {
            return Err(if password.is_empty() { err!("pdfdoc.password") } else { err!("pdfdoc.badPassword") });
        }
        let root = doc.catalog().and_then(|c| c.get(b"Pages")).and_then(|p| p.as_reference()).map_err(lerr)?;
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        // Flatten the page tree.
        for &p in &pages {
            let inh = inherited(&doc, p);
            let d = doc.get_dictionary_mut(p).map_err(lerr)?;
            for (k, v) in inh {
                d.set(k, v);
            }
        }
        let rd = doc.get_dictionary_mut(root).map_err(lerr)?;
        for k in INHERITED {
            rd.remove(k);
        }
        let mut pd = PdfDoc::with(doc, root, pages, path);
        pd.rebuild_tree();
        Ok(pd)
    }

    fn rebuild_tree(&mut self) {
        let kids: Vec<Object> = self.pages.iter().map(|p| Object::Reference(*p)).collect();
        let n = kids.len() as i64;
        if let Ok(d) = self.doc.get_dictionary_mut(self.root) {
            d.set("Kids", Object::Array(kids));
            d.set("Count", n);
            d.remove(b"Parent");
        }
        for &p in &self.pages {
            if let Ok(d) = self.doc.get_dictionary_mut(p) {
                d.set("Parent", Object::Reference(self.root));
            }
        }
    }

    /// The visible box of page (CropBox or MediaBox, in pt) and its rotation.
    fn page_box(&self, page: ObjectId) -> ([f64; 4], i64) {
        let doc = &self.doc;
        let d = doc.get_dictionary(page).ok();
        let read = |k: &[u8]| -> Option<[f64; 4]> {
            let a = deref(doc, d?.get(k).ok()?).as_array().ok()?;
            if a.len() != 4 {
                return None;
            }
            let v: Vec<f64> = a.iter().map(|o| num(o, doc)).collect::<Option<_>>()?;
            Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
        };
        let mb = read(b"MediaBox").unwrap_or([0.0, 0.0, 595.276, 841.89]);
        let b = match read(b"CropBox") {
            // The crop box within the media box.
            Some(c) => [c[0].max(mb[0]), c[1].max(mb[1]), c[2].min(mb[2]), c[3].min(mb[3])],
            None => mb,
        };
        let rot = d.and_then(|d| d.get(b"Rotate").ok()).and_then(|o| num(o, doc)).unwrap_or(0.0) as i64;
        (b, rot.rem_euclid(360) / 90 * 90)
    }

    pub fn page_info(&self, i: usize) -> PageInfo {
        let (b, rot) = self.page_box(self.pages[i]);
        let (w, h) = (b[2] - b[0], b[3] - b[1]);
        let (w, h) = if rot % 180 == 0 { (w, h) } else { (h, w) };
        let r = |v: f64| (v / MM * 10.0).round() / 10.0;
        PageInfo { width: r(w), height: r(h), rotation: rot }
    }

    /// Copies pages (of src, in this order) into this document before the
    /// page at index at. The objects they use are copied too; links to
    /// pages that are not copied are dropped.
    pub fn insert_pages(&mut self, at: usize, src: &Document, pages: &[ObjectId]) {
        let src_pages: HashSet<ObjectId> = src.page_iter().collect();
        let mut map: HashMap<ObjectId, ObjectId> = HashMap::new();
        let mut queue: VecDeque<(ObjectId, ObjectId)> = VecDeque::new();
        let mut out = Vec::new();
        for &pid in pages {
            let Ok(d) = src.get_dictionary(pid) else { continue };
            let mut d = d.clone();
            for (k, v) in inherited(src, pid) {
                d.set(k, v);
            }
            for k in [&b"Parent"[..], b"StructParents", b"B"] {
                d.remove(k);
            }
            let id = self.doc.new_object_id();
            map.insert(pid, id);
            let obj = copy_obj(Object::Dictionary(d), src, &src_pages, &mut map, &mut queue, &mut self.doc);
            self.doc.objects.insert(id, obj);
            out.push(id);
        }
        while let Some((old, new)) = queue.pop_front() {
            let obj = src.get_object(old).cloned().unwrap_or(Object::Null);
            let obj = copy_obj(obj, src, &src_pages, &mut map, &mut queue, &mut self.doc);
            self.doc.objects.insert(new, obj);
        }
        let at = at.min(self.pages.len());
        self.pages.splice(at..at, out);
        self.restructured = true;
        self.rebuild_tree();
    }

    /// Inserts an empty page of w x h pt before the page at index at.
    pub fn insert_blank(&mut self, at: usize, w: f64, h: f64) {
        let mut d = Dictionary::new();
        d.set("Type", name("Page"));
        d.set("MediaBox", Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Real(w as f32), Object::Real(h as f32)]));
        d.set("Resources", Dictionary::new());
        let id = self.doc.add_object(d);
        let at = at.min(self.pages.len());
        self.pages.insert(at, id);
        self.restructured = true;
        self.rebuild_tree();
    }

    /// Removes the pages at these indexes.
    pub fn remove_pages(&mut self, idx: &[usize]) {
        let drop: HashSet<usize> = idx.iter().copied().collect();
        let removed: Vec<ObjectId> = self.pages.iter().enumerate().filter(|(i, _)| drop.contains(i)).map(|(_, p)| *p).collect();
        self.pages = self.pages.iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, p)| *p).collect();
        for p in removed {
            // Links (outlines) may still point at it, but it is no longer a page of the tree.
            if let Ok(d) = self.doc.get_dictionary_mut(p) {
                d.remove(b"Parent");
            }
        }
        self.restructured = true;
        self.rebuild_tree();
    }

    /// Turns the pages at these indexes clockwise by deg (a multiple of 90).
    pub fn rotate_pages(&mut self, idx: &[usize], deg: i64) {
        for &i in idx {
            let p = self.pages[i];
            let (_, rot) = self.page_box(p);
            if let Ok(d) = self.doc.get_dictionary_mut(p) {
                d.set("Rotate", (rot + deg).rem_euclid(360));
            }
        }
    }

    /// The fonts of addText: the named ones (all must exist), or else the
    /// defaults that exist (loaded only when a character needs them).
    fn fonts_for(names: &[String]) -> Result<FontList> {
        if names.is_empty() {
            let names = font::default_names();
            let mut list = FontList { names, files: Vec::new() };
            if (0..list.names.len()).all(|k| list.get(k).is_none()) {
                return Err(err!("pdfdoc.noDefaultFont"));
            }
            return Ok(list);
        }
        let files = names.iter().map(|n| resolve_font(n).map(|f| Some(Some(f))).ok_or_else(|| err!("pdfdoc.noFont", n))).collect::<Result<_>>()?;
        Ok(FontList { names: names.to_vec(), files })
    }

    /// The (unsealed) FontUse of a face.
    fn font_use(&mut self, file: &Arc<FontFile>) -> usize {
        if let Some(i) = self.fonts.iter().position(|u| !u.sealed && Arc::ptr_eq(&u.file, file)) {
            return i;
        }
        let id = self.doc.new_object_id();
        self.fonts.push(FontUse::new(file.clone(), id));
        self.fonts.len() - 1
    }

    /// The page's resources as its own dictionary, with the subdictionary
    /// key (Font, ExtGState) as its own too, so adding to them changes no
    /// other page.
    fn own_resource(&mut self, page: ObjectId, key: &[u8]) -> Result<&mut Dictionary> {
        let res = {
            let d = self.doc.get_dictionary(page).map_err(lerr)?;
            let mut res = match d.get(b"Resources").map(|o| deref(&self.doc, o)) {
                Ok(Object::Dictionary(r)) => r.clone(),
                _ => Dictionary::new(),
            };
            let sub = match res.get(key).map(|o| deref(&self.doc, o)) {
                Ok(Object::Dictionary(s)) => s.clone(),
                _ => Dictionary::new(),
            };
            res.set(key, sub);
            res
        };
        let d = self.doc.get_dictionary_mut(page).map_err(lerr)?;
        d.set("Resources", res);
        let Ok(Object::Dictionary(r)) = d.get_mut(b"Resources") else { unreachable!() };
        let Ok(Object::Dictionary(s)) = r.get_mut(key) else { unreachable!() };
        Ok(s)
    }

    /// Puts obj into the page's resources under key, returning its name.
    fn add_resource(&mut self, page: ObjectId, key: &[u8], prefix: &str, obj: Object) -> Result<String> {
        let sub = self.own_resource(page, key)?;
        if let Some((k, _)) = sub.iter().find(|(_, v)| **v == obj) {
            return Ok(String::from_utf8_lossy(k).into_owned());
        }
        let mut n = 1;
        let name = loop {
            let k = format!("{prefix}{n}");
            if !sub.has(k.as_bytes()) {
                break k;
            }
            n += 1;
        };
        sub.set(name.as_bytes(), obj);
        Ok(name)
    }

    /// Appends content to the page, after putting its own content in q ... Q
    /// (so a graphics state it leaves changed does not move the new content).
    fn append_content(&mut self, page: ObjectId, content: Vec<u8>) -> Result<()> {
        let first = self.wrapped.insert(page);
        let mut content = content;
        let mut items = Vec::new();
        let old = self.doc.get_dictionary(page).map_err(lerr)?.get(b"Contents").ok().cloned();
        if first {
            let q = self.doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
            items.push(Object::Reference(q));
            content.splice(0..0, b"Q\n".iter().copied());
        }
        match old.as_ref().map(|o| (o, deref(&self.doc, o))) {
            Some((_, Object::Array(a))) => items.extend(a.iter().cloned()),
            Some((r @ Object::Reference(_), _)) => items.push(r.clone()),
            _ => {}
        }
        let id = self.doc.add_object(font::compressed(Dictionary::new(), content));
        items.push(Object::Reference(id));
        self.doc.get_dictionary_mut(page).map_err(lerr)?.set("Contents", Object::Array(items));
        Ok(())
    }

    /// Writes text (lines split by "\n") on the page at index i.
    pub fn add_text(&mut self, i: usize, text: &str, o: &TextOptions) -> Result<()> {
        let page = self.pages[i];
        let color = parse_color(&o.color).ok_or_else(|| err!("pdfdoc.badColor", &o.color))?;
        let mut fonts = Self::fonts_for(&o.font)?;
        let size = o.size;
        let lh = size * o.line_height;
        let first = fonts.first();
        let primary = first.face();
        let upm = primary.units_per_em() as f64;
        let ascent = primary.ascender() as f64 / upm * size;
        let descent = -(primary.descender() as f64) / upm * size;

        // Lines as runs of one font: (font use, CIDs), and the line width in pt.
        let mut glyphs: HashMap<char, (Arc<FontFile>, u16, f64)> = HashMap::new();
        let mut lines: Vec<(Vec<(usize, Vec<u16>)>, f64)> = Vec::new();
        for line in text.replace("\r\n", "\n").split(['\n', '\r']) {
            let mut runs: Vec<(usize, Vec<u16>)> = Vec::new();
            let mut width = 0.0;
            for c in line.chars() {
                let c = if c == '\t' { ' ' } else { c };
                if c.is_control() {
                    continue;
                }
                let (file, gid, adv) = glyphs.entry(c).or_insert_with(|| fonts.glyph(c)).clone();
                width += adv * size;
                let u = self.font_use(&file);
                let cid = self.fonts[u].cid(gid, &c.to_string());
                match runs.last_mut() {
                    Some((lu, cids)) if *lu == u => cids.push(cid),
                    _ => runs.push((u, vec![cid])),
                }
            }
            lines.push((runs, width));
        }

        let (b, rot) = self.page_box(page);
        let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
        let (w, h) = if rot % 180 == 0 { (bw, bh) } else { (bh, bw) };
        // Visible coordinates (pt from the bottom left as shown) to the page's.
        let m = match rot {
            90 => [0.0, 1.0, -1.0, 0.0, b[2], b[1]],
            180 => [-1.0, 0.0, 0.0, -1.0, b[2], b[3]],
            270 => [0.0, -1.0, 1.0, 0.0, b[0], b[3]],
            _ => [1.0, 0.0, 0.0, 1.0, b[0], b[1]],
        };
        let block_h = ascent + descent + lh * (lines.len().saturating_sub(1)) as f64;
        let (ax, align) = match (o.x, o.right) {
            (Some(x), _) => (x * MM, o.align),
            (None, Some(r)) => (w - r * MM, if o.align == Align::Left { Align::Right } else { o.align }),
            (None, None) => (
                match o.align {
                    Align::Left => 0.0,
                    Align::Center => w / 2.0,
                    Align::Right => w,
                },
                o.align,
            ),
        };
        // The top of the block, from the bottom of the page.
        let top = match (o.y, o.bottom) {
            (Some(y), _) => h - y * MM,
            (None, Some(bt)) => bt * MM + block_h,
            (None, None) => h,
        };

        let mut s = String::from("q\n");
        s.push_str(&format!("{} {} {} {} {} {} cm\n", f(m[0]), f(m[1]), f(m[2]), f(m[3]), f(m[4]), f(m[5])));
        if o.opacity < 1.0 {
            let a = o.opacity.clamp(0.0, 1.0);
            let mut gs = Dictionary::new();
            gs.set("Type", name("ExtGState"));
            gs.set("ca", Object::Real(a as f32));
            gs.set("CA", Object::Real(a as f32));
            let n = self.add_resource(page, b"ExtGState", "XljsGS", Object::Dictionary(gs))?;
            s.push_str(&format!("/{n} gs\n"));
        }
        if o.rotate != 0.0 {
            let (sn, cs) = o.rotate.to_radians().sin_cos();
            s.push_str(&format!("1 0 0 1 {} {} cm\n{} {} {} {} 0 0 cm\n1 0 0 1 {} {} cm\n", f(ax), f(top), f(cs), f(sn), f(-sn), f(cs), f(-ax), f(-top)));
        }
        s.push_str(&format!("{} {} {} rg\nBT\n", f(color[0]), f(color[1]), f(color[2])));
        let mut names: HashMap<usize, String> = HashMap::new();
        for (k, (runs, lw)) in lines.iter().enumerate() {
            let base = top - ascent - lh * k as f64;
            let mut x = match align {
                Align::Left => ax,
                Align::Center => ax - lw / 2.0,
                Align::Right => ax - lw,
            };
            for (u, cids) in runs {
                let fname = match names.get(u) {
                    Some(n) => n.clone(),
                    None => {
                        let n = self.add_resource(page, b"Font", "XljsF", Object::Reference(self.fonts[*u].id))?;
                        names.insert(*u, n.clone());
                        n
                    }
                };
                let hex: String = cids.iter().map(|c| format!("{c:04X}")).collect();
                s.push_str(&format!("/{fname} {} Tf\n1 0 0 1 {} {} Tm\n<{hex}> Tj\n", f(size), f(x), f(base)));
                x += self.fonts[*u].run_width(cids) * size / 1000.0;
            }
        }
        s.push_str("ET\nQ\n");
        self.append_content(page, s.into_bytes())
    }

    /// Embeds the fonts used and writes the page tree.
    pub fn finish(&mut self) -> Result<()> {
        for u in self.fonts.iter_mut().filter(|u| !u.sealed) {
            u.embed(&mut self.doc)?;
        }
        self.rebuild_tree();
        if self.restructured
            && let Ok(c) = self.doc.catalog_mut()
        {
            c.remove(b"PageLabels");
        }
        Ok(())
    }

    /// The document as PDF bytes.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        if self.pages.is_empty() {
            return Err(err!("pdfdoc.noPages"));
        }
        self.finish()?;
        self.doc.prune_objects();
        let mut out = Vec::new();
        self.doc.save_to(&mut out).map_err(|e| err!("pdfdoc.invalid", e.to_string()))?;
        Ok(out)
    }
}

/// Copies obj of src into dst: references become new objects (queued to
/// be copied), except pages that are not copied, the page tree and the
/// catalog (dropped).
fn copy_obj(
    obj: Object,
    src: &Document,
    src_pages: &HashSet<ObjectId>,
    map: &mut HashMap<ObjectId, ObjectId>,
    queue: &mut VecDeque<(ObjectId, ObjectId)>,
    dst: &mut Document,
) -> Object {
    match obj {
        Object::Reference(id) => {
            if let Some(n) = map.get(&id) {
                return Object::Reference(*n);
            }
            let Ok(target) = src.get_object(id) else { return Object::Null };
            let ty = match target {
                Object::Dictionary(d) => d.get(b"Type").ok().and_then(|t| t.as_name().ok()).map(<[u8]>::to_vec),
                _ => None,
            };
            if src_pages.contains(&id) || matches!(ty.as_deref(), Some(b"Pages" | b"Catalog")) {
                return Object::Null;
            }
            let n = dst.new_object_id();
            map.insert(id, n);
            queue.push_back((id, n));
            Object::Reference(n)
        }
        Object::Array(a) => Object::Array(a.into_iter().map(|o| copy_obj(o, src, src_pages, map, queue, dst)).collect()),
        Object::Dictionary(d) => Object::Dictionary(copy_dict(d, src, src_pages, map, queue, dst)),
        Object::Stream(mut s) => {
            s.dict = copy_dict(std::mem::take(&mut s.dict), src, src_pages, map, queue, dst);
            Object::Stream(s)
        }
        o => o,
    }
}

fn copy_dict(
    d: Dictionary,
    src: &Document,
    src_pages: &HashSet<ObjectId>,
    map: &mut HashMap<ObjectId, ObjectId>,
    queue: &mut VecDeque<(ObjectId, ObjectId)>,
    dst: &mut Document,
) -> Dictionary {
    let mut out = Dictionary::new();
    for (k, v) in d.into_iter() {
        out.set(k, copy_obj(v, src, src_pages, map, queue, dst));
    }
    out
}

#[cfg(test)]
mod tests;
