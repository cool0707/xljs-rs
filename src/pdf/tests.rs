use super::*;
use crate::testutil::{lock_lang, repo_path};

fn three() -> PdfDoc {
    let data = std::fs::read(repo_path("testdata/pdf/three.pdf")).unwrap();
    PdfDoc::open(&data, "", String::new()).unwrap()
}

fn reload(d: &mut PdfDoc) -> Document {
    Document::load_mem(&d.to_bytes().unwrap()).unwrap()
}

/// The text of each page (lopdf's extraction, through ToUnicode).
fn texts(doc: &Document) -> Vec<String> {
    (1..=doc.get_pages().len() as u32).map(|n| doc.extract_text(&[n]).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")).collect()
}

/// A font of the system with Latin letters (None: skip the test).
fn latin_font() -> Option<String> {
    ["DejaVu Sans", "Arial", "Liberation Sans", "Helvetica", "Yu Gothic", "IPAGothic"].into_iter().find(|n| font::locate(n).is_some()).map(String::from)
}

fn japanese_font() -> Option<String> {
    ["Yu Gothic", "MS Gothic", "IPAGothic", "IPAexGothic", "Noto Sans CJK JP", "Hiragino Sans"].into_iter().find(|n| font::locate(n).is_some()).map(String::from)
}

/// A PDF whose pages inherit MediaBox, Resources and Rotate from a nested
/// page tree.
fn nested() -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let root = doc.new_object_id();
    let mid = doc.new_object_id();
    let font = {
        let mut f = Dictionary::new();
        f.set("Type", name("Font"));
        f.set("Subtype", name("Type1"));
        f.set("BaseFont", name("Helvetica"));
        doc.add_object(f)
    };
    let mut res = Dictionary::new();
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font));
    res.set("Font", fonts);
    let res_id = doc.add_object(res);
    let mut kids = Vec::new();
    for (i, parent) in [(1, root), (2, mid), (3, mid)] {
        let c = doc.add_object(Stream::new(Dictionary::new(), format!("BT /F1 12 Tf 72 700 Td (Nested {i}) Tj ET").into_bytes()));
        let mut p = Dictionary::new();
        p.set("Type", name("Page"));
        p.set("Parent", Object::Reference(parent));
        p.set("Contents", Object::Reference(c));
        kids.push((parent, doc.add_object(p)));
    }
    let mut m = Dictionary::new();
    m.set("Type", name("Pages"));
    m.set("Parent", Object::Reference(root));
    m.set("Kids", Object::Array(kids.iter().filter(|(p, _)| *p == mid).map(|(_, k)| Object::Reference(*k)).collect()));
    m.set("Count", 2);
    m.set("Rotate", 90);
    m.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 420.into(), 595.into()]));
    doc.objects.insert(mid, Object::Dictionary(m));
    let mut r = Dictionary::new();
    r.set("Type", name("Pages"));
    r.set("Kids", Object::Array(vec![Object::Reference(kids[0].1), Object::Reference(mid)]));
    r.set("Count", 3);
    r.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 595.into(), 842.into()]));
    r.set("Resources", Object::Reference(res_id));
    doc.objects.insert(root, Object::Dictionary(r));
    let mut cat = Dictionary::new();
    cat.set("Type", name("Catalog"));
    cat.set("Pages", Object::Reference(root));
    let cat = doc.add_object(cat);
    doc.trailer.set("Root", Object::Reference(cat));
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn opens_and_flattens_the_page_tree() {
    let _g = lock_lang("en");
    let mut d = PdfDoc::open(&nested(), "", String::new()).unwrap();
    assert_eq!(d.pages.len(), 3);
    assert_eq!(d.page_info(0), PageInfo { width: 209.9, height: 297.0, rotation: 0 });
    assert_eq!(d.page_info(1), PageInfo { width: 209.9, height: 148.2, rotation: 90 });
    // Move the rotated page to the front: it keeps what it inherited.
    let copy = d.doc.clone();
    let p = d.pages[2];
    d.insert_pages(0, &copy, &[p]);
    d.remove_pages(&[3]);
    let doc = reload(&mut d);
    assert_eq!(texts(&doc), ["Nested 3", "Nested 1", "Nested 2"]);
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let first = doc.get_dictionary(pages[0]).unwrap();
    assert_eq!(first.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
    assert!(first.has(b"Resources") && first.has(b"MediaBox"));
}

#[test]
fn joins_removes_and_rotates_pages() {
    let _g = lock_lang("en");
    let mut d = three();
    assert_eq!(d.page_info(2), PageInfo { width: 210.0, height: 148.0, rotation: 90 });
    let src = three();
    d.insert_pages(1, &src.doc, &[src.pages[2], src.pages[0]]);
    d.insert_blank(5, 100.0 * MM, 50.0 * MM);
    d.remove_pages(&[0]);
    d.rotate_pages(&[0, 1], 90);
    assert_eq!(d.pages.len(), 5);
    assert_eq!(d.page_info(0).rotation, 180);
    assert_eq!(d.page_info(1).rotation, 90);
    assert_eq!(d.page_info(4), PageInfo { width: 100.0, height: 50.0, rotation: 0 });
    let doc = reload(&mut d);
    assert_eq!(texts(&doc), ["Page 3", "Page 1", "Page 2", "Page 3", ""]);
    // The outline of the copied pages is not taken along.
    assert!(doc.objects.len() < 60);
}

#[test]
fn reads_encrypted_files() {
    let _g = lock_lang("en");
    let data = std::fs::read(repo_path("testdata/pdf/secret.pdf")).unwrap();
    let e = PdfDoc::open(&data, "", String::new()).err().unwrap();
    assert!(e.to_string().contains("password"), "{e}");
    let e = PdfDoc::open(&data, "wrong", String::new()).err().unwrap();
    assert!(e.to_string().contains("wrong"), "{e}");
    let mut d = PdfDoc::open(&data, "secret", String::new()).unwrap();
    // Saved without the password.
    assert_eq!(texts(&reload(&mut d)), ["Secret"]);
    assert!(PdfDoc::open(b"hello", "", String::new()).is_err());
}

#[test]
fn adds_text() {
    let _g = lock_lang("en");
    let Some(f) = latin_font() else {
        eprintln!("no font: skipped");
        return;
    };
    let mut d = three();
    let o = TextOptions { font: vec![f.clone()], x: Some(20.0), y: Some(20.0), ..Default::default() };
    d.add_text(0, "Hello", &o).unwrap();
    let o = TextOptions { font: vec![f], bottom: Some(10.0), align: Some(Align::Center), color: "#336699".into(), opacity: 0.5, rotate: 45.0, ..Default::default() };
    d.add_text(2, "World\nAgain", &o).unwrap();
    let doc = reload(&mut d);
    let t = texts(&doc);
    assert!(t[0].contains("Hello") && t[0].contains("Page 1"), "{t:?}");
    assert!(t[2].contains("World") && t[2].contains("Again"), "{t:?}");
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let content = String::from_utf8_lossy(&doc.get_page_content(pages[2])).into_owned();
    // The rotated page (90): visible coordinates turned into the page's.
    assert!(content.contains("0 1 -1 0 419.528 0 cm"), "{content}");
    assert!(content.contains("0.2 0.4 0.6 rg") && content.contains(" gs\n"), "{content}");
    assert!(content.starts_with("q\n"), "{content}");
    // One embedded subset, shared by the pages.
    let fonts: Vec<_> = doc.objects.values().filter_map(|o| o.as_dict().ok()).filter(|d| d.has(b"FontFile2") || d.has(b"FontFile3")).collect();
    assert_eq!(fonts.len(), 1);
}

#[test]
fn adds_japanese_text_with_fallback() {
    let _g = lock_lang("ja");
    let (Some(latin), Some(ja)) = (latin_font(), japanese_font()) else {
        eprintln!("no font: skipped");
        return;
    };
    let mut d = PdfDoc::create();
    d.insert_blank(0, 595.0, 842.0);
    // Latin letters from the first font, kana and kanji from the second.
    let o = TextOptions { font: vec![latin, ja], ..Default::default() };
    d.add_text(0, "社外秘 ABC", &o).unwrap();
    let doc = reload(&mut d);
    assert_eq!(texts(&doc)[0].replace(' ', ""), "社外秘ABC");
}

#[test]
fn embeds_cff_fonts() {
    let _g = lock_lang("en");
    let Some(otf) = walkdir::WalkDir::new("/usr/share/fonts").into_iter().flatten().map(|e| e.into_path()).find(|p| {
        p.extension().is_some_and(|e| e == "otf") && std::fs::read(p).is_ok_and(|b| ttf_parser::Face::parse(&b, 0).is_ok_and(|f| f.tables().cff.is_some()))
    }) else {
        eprintln!("no CFF font: skipped");
        return;
    };
    let mut d = PdfDoc::create();
    d.insert_blank(0, 595.0, 842.0);
    d.add_text(0, "Hello", &TextOptions { font: vec![otf.to_string_lossy().into_owned()], ..Default::default() }).unwrap();
    let doc = reload(&mut d);
    assert!(doc.objects.values().any(|o| o.as_dict().is_ok_and(|d| d.has(b"FontFile3"))));
    assert_eq!(texts(&doc)[0], "Hello");
}

#[test]
fn errors() {
    let _g = lock_lang("en");
    let mut d = PdfDoc::create();
    assert!(d.to_bytes().err().unwrap().to_string().contains("no pages"));
    d.insert_blank(0, 595.0, 842.0);
    let o = TextOptions { font: vec!["No Such Font 123".into()], ..Default::default() };
    assert!(d.add_text(0, "x", &o).err().unwrap().to_string().contains("No Such Font 123"));
    let o = TextOptions { color: "#12".into(), ..Default::default() };
    assert!(d.add_text(0, "x", &o).is_err());
}

#[test]
fn colors() {
    assert_eq!(parse_color(""), Some([0.0, 0.0, 0.0]));
    assert_eq!(parse_color("#f00"), Some([1.0, 0.0, 0.0]));
    assert_eq!(parse_color("#336699"), Some([0.2, 0.4, 0.6]));
    assert_eq!(parse_color("Gray").map(|c| (c[0] * 255.0).round()), Some(128.0));
    assert_eq!(parse_color("#ggg"), None);
    assert_eq!(parse_color("teal"), None);
}

/// The x and y of each Tm in the content of the page at index i.
fn text_positions(doc: &Document, i: usize) -> Vec<(f64, f64)> {
    let page = doc.get_pages().into_values().nth(i).unwrap();
    let c = String::from_utf8_lossy(&doc.get_page_content(page)).into_owned();
    let re = regex::Regex::new(r"1 0 0 1 (\S+) (\S+) Tm").unwrap();
    re.captures_iter(&c).map(|m| (m[1].parse().unwrap(), m[2].parse().unwrap())).collect()
}

#[test]
fn places_text_by_position() {
    let _g = lock_lang("en");
    let Some(f) = latin_font() else {
        eprintln!("no font: skipped");
        return;
    };
    let (w, h) = (595.276, 841.89);
    let mut d = PdfDoc::create();
    let positions = ["top-left", "top-center", "top-right", "center", "bottom-left", "bottom-center", "bottom-right"];
    for (i, p) in positions.iter().enumerate() {
        d.insert_blank(i, w, h);
        let o = TextOptions { font: vec![f.clone()], position: Some(p.to_string()), ..Default::default() };
        d.add_text(i, "No. 12", &o).unwrap();
    }
    // Margins of 20 mm across and 5 mm down; x and y win over the position.
    d.insert_blank(7, w, h);
    let o = TextOptions { font: vec![f.clone()], position: Some("bottom-right".into()), margin: [20.0, 5.0], ..Default::default() };
    d.add_text(7, "No. 12", &o).unwrap();
    let o = TextOptions { font: vec![f.clone()], position: Some("bottom-right".into()), x: Some(50.0), ..Default::default() };
    d.add_text(7, "No. 12", &o).unwrap();
    let doc = reload(&mut d);
    let p: Vec<(f64, f64)> = (0..7).map(|i| text_positions(&doc, i)[0]).collect();
    let m = 10.0 * MM;
    let width = w - m - p[2].0;
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    assert!(near(p[0].0, m) && near(p[4].0, m), "{p:?}");
    assert!(near(p[1].0, (w - width) / 2.0) && near(p[5].0, p[1].0) && near(p[3].0, p[1].0), "{p:?}");
    assert!(near(p[6].0, p[2].0), "{p:?}");
    assert!(near(p[0].1, p[1].1) && near(p[0].1, p[2].1) && p[0].1 < h - m && p[0].1 > h - m - 11.0, "{p:?}");
    assert!(near(p[4].1, p[5].1) && near(p[4].1, p[6].1) && p[4].1 > m && p[4].1 < m + 5.0, "{p:?}");
    assert!(p[3].1 > h / 2.0 - 11.0 && p[3].1 < h / 2.0, "{p:?}");
    let q = text_positions(&doc, 7);
    assert!(near(q[0].0, w - 20.0 * MM - width) && near(q[0].1 - p[4].1, 5.0 * MM - m), "{q:?}");
    assert!(near(q[1].0, 50.0 * MM - width) && near(q[1].1, p[4].1), "{q:?}");
    // Lines are aligned like the position (right), the block placed as a whole.
    let mut d = PdfDoc::create();
    d.insert_blank(0, w, h);
    let o = TextOptions { font: vec![f], position: Some("bottom-right".into()), ..Default::default() };
    d.add_text(0, "No. 12\n1", &o).unwrap();
    let doc = reload(&mut d);
    let r = text_positions(&doc, 0);
    assert!(near(r[0].0, p[6].0) && r[1].0 > r[0].0 && near(r[1].1, p[6].1) && r[0].1 > r[1].1, "{r:?}");
    assert!(parse_position("middle").is_none() && parse_position("center-left") == Some((Align::Left, VAlign::Middle)));
}
