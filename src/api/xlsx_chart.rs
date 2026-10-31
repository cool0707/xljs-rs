//! Excel's own charts in a workbook: where each chart is on its sheet, its
//! kind, series and axes, with the values of the series read from the
//! cells they refer to (or, when those cannot be read, from the chart's
//! cache). The script turns them into a ChartSpec (ExcelChart in
//! prelude.js).
//!
//! The chart parts are read from the xlsx file itself (kept when the
//! workbook is opened): umya-spreadsheet's model of a chart leaves out
//! kinds it does not know (stock, surface, the newer chartEx kinds) and
//! keeps one chart group of each kind.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::{BytesStart, Event};
use serde_json::{Map, Value as J, json};
use umya_spreadsheet::CellRawValue;

use super::xlsx::{Book, date_format_code, format_code, from_serial, parse_ref, style_is_date};

/// The parts of an xlsx file that describe its drawings (charts, shapes,
/// pictures) and how sheets reach them. Empty when the file has no drawings.
#[derive(Default, Debug, Clone)]
pub struct ChartParts {
    pub(super) files: HashMap<String, Vec<u8>>,
}

impl ChartParts {
    pub fn collect(data: &[u8]) -> ChartParts {
        let mut files = HashMap::new();
        let Ok(mut zr) = zip::ZipArchive::new(Cursor::new(data)) else { return ChartParts { files } };
        // Charts, shapes and pictures all live in drawings.
        if !zr.file_names().any(|n| n.starts_with("xl/drawings/")) {
            return ChartParts { files };
        }
        for i in 0..zr.len() {
            let Ok(mut f) = zr.by_index(i) else { continue };
            let name = f.name().to_string();
            let keep = name == "xl/workbook.xml"
                || name == "xl/_rels/workbook.xml.rels"
                || name.starts_with("xl/worksheets/_rels/")
                || name.starts_with("xl/drawings/")
                || name.starts_with("xl/charts/")
                || name.starts_with("xl/media/")
                || name.starts_with("xl/theme/");
            let mut b = Vec::new();
            if keep && f.read_to_end(&mut b).is_ok() {
                files.insert(name, b);
            }
        }
        ChartParts { files }
    }

    pub(super) fn xml(&self, part: &str) -> Option<Node> {
        parse(self.files.get(part)?)
    }

    /// The relationships of a part: id -> (type, part it points to).
    pub(super) fn rels(&self, part: &str) -> HashMap<String, (String, String)> {
        let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
        let mut out = HashMap::new();
        let Some(root) = self.xml(&format!("{dir}/_rels/{file}.rels")) else { return out };
        for r in root.all("Relationship") {
            let (Some(id), Some(target)) = (r.attr("Id"), r.attr("Target")) else { continue };
            if r.attr("TargetMode") == Some("External") {
                continue;
            }
            out.insert(id.to_string(), (r.attr("Type").unwrap_or("").to_string(), resolve(dir, target)));
        }
        out
    }

    /// The part of the worksheet named sheet.
    pub(super) fn sheet_part(&self, sheet: &str) -> Option<String> {
        let wb = self.xml("xl/workbook.xml")?;
        let s = wb.child("sheets")?.all("sheet").find(|s| s.attr("name") == Some(sheet))?;
        let id = s.attr("id")?;
        self.rels("xl/workbook.xml").remove(id).map(|(_, p)| p)
    }

    /// The theme's colors by name (accent1, dk1, ...) as RRGGBB.
    pub(super) fn theme(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let Some(t) = self.xml("xl/theme/theme1.xml") else { return out };
        if let Some(cs) = t.find("clrScheme") {
            for c in &cs.children {
                let v = c.child("srgbClr").and_then(|x| x.attr("val")).or_else(|| c.child("sysClr").and_then(|x| x.attr("lastClr")));
                if let Some(v) = v {
                    out.insert(c.name.clone(), v.to_uppercase());
                }
            }
        }
        out
    }
}

/// target relative to the folder dir ("../drawings/drawing1.xml"), or absolute ("/xl/...").
pub(super) fn resolve(dir: &str, target: &str) -> String {
    if let Some(t) = target.strip_prefix('/') {
        return t.to_string();
    }
    let mut parts: Vec<&str> = if dir.is_empty() { vec![] } else { dir.split('/').collect() };
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

// ---- a small XML tree (local names only) ----

#[derive(Default, Debug)]
pub(super) struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// The val attribute of the child named name.
    fn val(&self, name: &str) -> Option<&str> {
        self.child(name).and_then(|c| c.attr("val"))
    }
    /// A boolean child (<c:smooth val="1"/>; no val means true).
    fn flag(&self, name: &str) -> Option<bool> {
        self.child(name).map(|c| !matches!(c.attr("val"), Some("0" | "false")))
    }
    pub fn path(&self, names: &[&str]) -> Option<&Node> {
        names.iter().try_fold(self, |n, name| n.child(name))
    }
    /// The first descendant (or self) named name.
    pub fn find(&self, name: &str) -> Option<&Node> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }
    /// The text of the a:t runs, paragraphs joined by a line break.
    fn rich_text(&self) -> String {
        fn runs(n: &Node, out: &mut String) {
            if n.name == "t" {
                out.push_str(&n.text);
            }
            for c in &n.children {
                runs(c, out);
            }
        }
        let ps: Vec<String> = self
            .find("rich")
            .map(|r| {
                r.all("p")
                    .map(|p| {
                        let mut s = String::new();
                        runs(p, &mut s);
                        s
                    })
                    .collect()
            })
            .unwrap_or_default();
        ps.join("\n").trim().to_string()
    }
}

fn local(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    s.rsplit(':').next().unwrap_or("").to_string()
}

fn element(e: &BytesStart) -> Node {
    #[allow(deprecated)]
    let attrs = e
        .attributes()
        .flatten()
        .map(|a| (local(a.key.as_ref()), a.unescape_value().map(|v| v.into_owned()).unwrap_or_default()))
        .collect();
    Node { name: local(e.name().as_ref()), attrs, ..Default::default() }
}

pub(super) fn parse(xml: &[u8]) -> Option<Node> {
    let mut r = quick_xml::Reader::from_reader(xml);
    let mut stack = vec![Node::default()];
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => stack.push(element(&e)),
            Ok(Event::Empty(e)) => {
                let n = element(&e);
                stack.last_mut()?.children.push(n);
            }
            Ok(Event::End(_)) => {
                let n = stack.pop()?;
                stack.last_mut()?.children.push(n);
            }
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.xml10_content() {
                    stack.last_mut()?.text.push_str(&s);
                }
            }
            Ok(Event::CData(t)) => {
                if let Ok(s) = t.decode() {
                    stack.last_mut()?.text.push_str(&s);
                }
            }
            Ok(Event::GeneralRef(g)) => {
                let s = match g.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match g.decode().as_deref() {
                        Ok("amp") => "&".into(),
                        Ok("lt") => "<".into(),
                        Ok("gt") => ">".into(),
                        Ok("quot") => "\"".into(),
                        Ok("apos") => "'".into(),
                        _ => String::new(),
                    },
                };
                stack.last_mut()?.text.push_str(&s);
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
    stack.pop()?.children.into_iter().next()
}

// ---- values ----

/// A date as the script receives it: { "$date": [y, m (1-12), d, h, mi, s, ms] }.
fn date_json(t: (i32, u32, u32, u32, u32, u32, u32)) -> J {
    json!({ "$date": [t.0, t.1, t.2, t.3, t.4, t.5, t.6] })
}

fn cell_json(b: &Book, c: Option<&umya_spreadsheet::Cell>) -> J {
    let Some(c) = c else { return J::Null };
    match c.raw_value() {
        CellRawValue::Empty => J::Null,
        CellRawValue::Bool(v) => J::Bool(*v),
        CellRawValue::Numeric(f) => {
            if style_is_date(c.style().number_format())
                && let Some(t) = from_serial(*f, b.date1904)
            {
                return date_json(t);
            }
            json!(f)
        }
        CellRawValue::String(s) | CellRawValue::Lazy(s) => {
            if s.is_empty() { J::Null } else { J::String(s.to_string()) }
        }
        other => {
            let s = other.to_string();
            if s.is_empty() { J::Null } else { J::String(s) }
        }
    }
}

/// One area of a reference: (sheet, first column, first row, last column, last row).
type Area = (String, u32, u32, u32, u32);

/// The areas of a series reference ("'売上 2026'!$B$2:$B$13", "(Sheet1!$A$1,Sheet1!$A$3)").
fn areas(f: &str) -> Option<Vec<Area>> {
    let f = f.trim();
    let f = f.strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or(f);
    let mut parts = Vec::new();
    let (mut cur, mut quoted) = (String::new(), false);
    for ch in f.chars() {
        match ch {
            '\'' => {
                quoted = !quoted;
                cur.push(ch);
            }
            ',' if !quoted => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);
    parts
        .iter()
        .map(|p| {
            let (sheet, range) = p.rsplit_once('!')?;
            let sheet = sheet.trim();
            let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                Some(s) => s.replace("''", "'"),
                None => sheet.to_string(),
            };
            let range = range.replace('$', "");
            let (a, z) = range.split_once(':').unwrap_or((&range, &range));
            let (c1, r1) = parse_ref(a).ok()?;
            let (c2, r2) = parse_ref(z).ok()?;
            Some((sheet, c1.min(c2), r1.min(r2), c1.max(c2), r1.max(r2)))
        })
        .collect()
}

/// The cells of the areas in order: down a column, or along a row. Of an
/// area more than one cell wide and tall (categories with levels), the
/// innermost level: the last column (or row).
fn area_cells(a: &[Area]) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    for (s, c1, r1, c2, r2) in a {
        if c1 == c2 || r2 - r1 >= c2 - c1 {
            for r in *r1..=*r2 {
                out.push((s.clone(), *c2, r));
            }
        } else {
            for c in *c1..=*c2 {
                out.push((s.clone(), c, *r2));
            }
        }
    }
    out
}

/// The data of c:cat, c:val, c:xVal, c:yVal, c:bubbleSize or c:tx:
/// { values, format, ref, header } (header: the text of the cell above a
/// column of values, or left of a row). Text among values (not labels:
/// c:val, c:yVal, c:bubbleSize) is no value, as Excel draws nothing for it.
fn source(b: &Book, n: &Node) -> J {
    let labels = !matches!(n.name.as_str(), "val" | "yVal" | "bubbleSize");
    let r = n.children.iter().find(|c| matches!(c.name.as_str(), "numRef" | "strRef" | "multiLvlStrRef" | "numLit" | "strLit"));
    let Some(r) = r else { return J::Null };
    let formula = r.child("f").map(|f| f.text.trim().to_string()).unwrap_or_default();
    // The cache (and literals): values by point index.
    let cache = r.children.iter().find(|c| matches!(c.name.as_str(), "numCache" | "strCache" | "multiLvlStrCache")).unwrap_or(r);
    let cache = if cache.name == "multiLvlStrCache" { cache.all("lvl").next().unwrap_or(cache) } else { cache };
    let cache_format = cache.child("formatCode").map(|f| f.text.trim().to_string()).unwrap_or_default();
    let count = cache.val("ptCount").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let numeric = matches!(r.name.as_str(), "numRef" | "numLit");
    let mut cached = vec![J::Null; count];
    for p in cache.all("pt") {
        let idx = p.attr("idx").and_then(|v| v.parse::<usize>().ok()).unwrap_or(cached.len());
        if idx >= cached.len() {
            cached.resize(idx + 1, J::Null);
        }
        let text = p.child("v").map(|v| v.text.clone()).unwrap_or_default();
        let fmt = p.attr("formatCode").map(str::to_string).unwrap_or_else(|| cache_format.clone());
        cached[idx] = match text.trim().parse::<f64>() {
            Ok(f) if numeric => {
                if date_format_code(&fmt)
                    && let Some(t) = from_serial(f, b.date1904)
                {
                    date_json(t)
                } else {
                    json!(f)
                }
            }
            _ => J::String(text),
        };
    }
    let mut out = Map::new();
    let mut format = cache_format.clone();
    let mut values = cached.clone();
    if !formula.is_empty()
        && let Some(a) = areas(&formula)
    {
        let cells = area_cells(&a);
        if !cells.is_empty() && cells.iter().all(|(s, _, _)| b.wb.sheet_by_name(s).is_ok()) {
            values = cells
                .iter()
                .map(|(s, c, r)| {
                    let ws = b.wb.sheet_by_name(s).unwrap();
                    let v = cell_json(b, ws.cell((*c, *r)));
                    if !labels && v.is_string() { J::Null } else { v }
                })
                .collect();
            let (s, c, r) = &cells[0];
            if let Some(cell) = b.wb.sheet_by_name(s).unwrap().cell((*c, *r)) {
                let code = format_code(cell.style().number_format());
                if !code.is_empty() && code != "General" {
                    format = code;
                }
            }
            // The header: the cell before the first one (a column: above it; a row: left of it).
            if let [(s, c1, r1, c2, r2)] = a.as_slice() {
                let ws = b.wb.sheet_by_name(s).unwrap();
                let before = if c1 == c2 || r2 - r1 >= c2 - c1 {
                    if *r1 > 1 { ws.cell((*c2, r1 - 1)) } else { None }
                } else if *c1 > 1 {
                    ws.cell((c1 - 1, *r2))
                } else {
                    None
                };
                if let J::String(h) = cell_json(b, before) {
                    out.insert("header".into(), J::String(h));
                }
            }
        }
        out.insert("ref".into(), J::String(formula));
    }
    if format == "General" {
        format.clear();
    }
    out.insert("values".into(), J::Array(values));
    out.insert("format".into(), J::String(format));
    J::Object(out)
}

/// The text of a series name (c:tx) or title (c:tx with c:rich or c:strRef).
fn text_of(b: &Book, tx: &Node) -> Option<String> {
    if tx.find("rich").is_some() {
        return Some(tx.rich_text());
    }
    if let Some(v) = tx.child("v") {
        return Some(v.text.clone());
    }
    let s = source(b, tx);
    let parts: Vec<String> = s["values"]
        .as_array()?
        .iter()
        .filter_map(|v| match v {
            J::String(s) => Some(s.clone()),
            J::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect();
    Some(parts.join(" "))
}

// ---- colors ----

/// A color element (a:srgbClr, a:schemeClr, a:sysClr) as RRGGBB, before its modifiers.
fn base_color(c: &Node, theme: &HashMap<String, String>) -> Option<String> {
    Some(match c.name.as_str() {
        "srgbClr" => c.attr("val")?.to_uppercase(),
        "sysClr" => c.attr("lastClr")?.to_uppercase(),
        "prstClr" => match c.attr("val")? {
            "black" => "000000",
            "white" => "FFFFFF",
            "red" => "FF0000",
            "green" => "008000",
            "blue" => "0000FF",
            "yellow" => "FFFF00",
            "gray" | "grey" => "808080",
            _ => return None,
        }
        .to_string(),
        "schemeClr" => {
            let name = match c.attr("val")? {
                "tx1" => "dk1",
                "bg1" => "lt1",
                "tx2" => "dk2",
                "bg2" => "lt2",
                v => v,
            };
            theme.get(name).cloned().or_else(|| office_color(name).map(str::to_string))?
        }
        _ => return None,
    })
}

/// The color modifiers among the children of n (lumMod, lumOff, shade, tint) as fractions.
fn mods_of(n: &Node) -> Vec<(String, f64)> {
    n.children
        .iter()
        .filter(|c| matches!(c.name.as_str(), "lumMod" | "lumOff" | "shade" | "tint"))
        .filter_map(|c| Some((c.name.clone(), c.attr("val")?.parse::<f64>().ok()? / 100000.0)))
        .collect()
}

/// rgb (RRGGBB) with modifiers applied, as "#RRGGBB". lumMod / lumOff work
/// on the luminance (HSL), as Office does; shade / tint darken and lighten.
fn modified(rgb: &str, mods: &[(String, f64)]) -> Option<String> {
    let v = u32::from_str_radix(rgb, 16).ok()?;
    let (mut r, mut g, mut b) = (((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0);
    let get = |k: &str| mods.iter().find(|(n, _)| n == k).map(|(_, x)| *x);
    if get("lumMod").is_some() || get("lumOff").is_some() {
        let (h, s, l) = to_hsl(r, g, b);
        let l = (l * get("lumMod").unwrap_or(1.0) + get("lumOff").unwrap_or(0.0)).clamp(0.0, 1.0);
        (r, g, b) = from_hsl(h, s, l);
    }
    if let Some(x) = get("shade") {
        (r, g, b) = (r * x, g * x, b * x);
    }
    if let Some(x) = get("tint") {
        (r, g, b) = (1.0 - (1.0 - r) * x, 1.0 - (1.0 - g) * x, 1.0 - (1.0 - b) * x);
    }
    let f = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Some(format!("#{:02X}{:02X}{:02X}", f(r), f(g), f(b)))
}

fn to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

/// A color of a fill (a:solidFill and the like) as "#RRGGBB".
pub(super) fn color_of(fill: &Node, theme: &HashMap<String, String>) -> Option<String> {
    let c = fill.children.iter().find(|c| matches!(c.name.as_str(), "srgbClr" | "schemeClr" | "sysClr" | "prstClr"))?;
    modified(&base_color(c, theme)?, &mods_of(c))
}

/// The automatic colors of a chart's series and points, as Excel 2013 and
/// later picks them: the chart's color style (xl/charts/colorsN.xml, the
/// palette chosen with "Change Colors") or else the theme's accent1 to
/// accent6. After the last color they repeat with the next variation
/// (darker, lighter, ...). Monochrome styles are read the same way, which
/// is close to (not exactly) what Excel shows.
struct AutoColors {
    colors: Vec<String>,
    variations: Vec<Vec<(String, f64)>>,
}

impl AutoColors {
    fn new(parts: &ChartParts, chart_part: &str, theme: &HashMap<String, String>) -> AutoColors {
        let style = parts
            .rels(chart_part)
            .into_values()
            .find(|(t, _)| t.ends_with("/chartColorStyle"))
            .and_then(|(_, p)| parts.xml(&p));
        if let Some(st) = style {
            let colors: Vec<String> = st
                .children
                .iter()
                .filter(|c| matches!(c.name.as_str(), "srgbClr" | "schemeClr" | "sysClr"))
                .filter_map(|c| modified(&base_color(c, theme)?, &mods_of(c)).map(|x| x[1..].to_string()))
                .collect();
            if !colors.is_empty() {
                return AutoColors { colors, variations: st.all("variation").map(mods_of).collect() };
            }
        }
        // Office's default color style (colors1.xml of a new chart).
        let m = |v: &[(&str, f64)]| v.iter().map(|(k, x)| (k.to_string(), *x)).collect::<Vec<_>>();
        AutoColors {
            colors: (1..=6).map(|i| format!("accent{i}")).filter_map(|a| theme.get(&a).cloned().or_else(|| office_color(&a).map(str::to_string))).collect(),
            variations: vec![
                vec![],
                m(&[("lumMod", 0.6)]),
                m(&[("lumMod", 0.8), ("lumOff", 0.2)]),
                m(&[("lumMod", 0.8)]),
                m(&[("lumMod", 0.6), ("lumOff", 0.4)]),
                m(&[("lumMod", 0.5)]),
                m(&[("lumMod", 0.7), ("lumOff", 0.3)]),
                m(&[("lumMod", 0.7)]),
                m(&[("lumMod", 0.5), ("lumOff", 0.5)]),
            ],
        }
    }

    /// The color of the i-th series (its c:idx) or point.
    fn nth(&self, i: usize) -> Option<String> {
        let n = self.colors.len();
        if n == 0 {
            return None;
        }
        let none = Vec::new();
        let var = if self.variations.is_empty() { &none } else { &self.variations[(i / n) % self.variations.len()] };
        modified(&self.colors[i % n], var)
    }
}

/// Office's default theme (when the workbook has none).
fn office_color(name: &str) -> Option<&'static str> {
    Some(match name {
        "dk1" => "000000",
        "lt1" => "FFFFFF",
        "dk2" => "44546A",
        "lt2" => "E7E6E6",
        "accent1" => "4472C4",
        "accent2" => "ED7D31",
        "accent3" => "A5A5A5",
        "accent4" => "FFC000",
        "accent5" => "5B9BD5",
        "accent6" => "70AD47",
        _ => return None,
    })
}

/// { color, noFill } of a c:spPr: the fill, or the line for lines.
fn shape(sp: Option<&Node>, line: bool, theme: &HashMap<String, String>) -> (Option<String>, bool) {
    let Some(sp) = sp else { return (None, false) };
    let src = if line { sp.child("ln").unwrap_or(sp) } else { sp };
    if src.child("noFill").is_some() {
        return (None, true);
    }
    let color = src.child("solidFill").and_then(|f| color_of(f, theme));
    let color = color.or_else(|| src.child("gradFill").and_then(|g| g.find("gs")).and_then(|gs| color_of(gs, theme)));
    (color, false)
}

// ---- charts ----

const GROUPS: &[&str] = &[
    "barChart", "bar3DChart", "lineChart", "line3DChart", "pieChart", "pie3DChart", "ofPieChart", "doughnutChart",
    "areaChart", "area3DChart", "scatterChart", "bubbleChart", "radarChart", "stockChart", "surfaceChart", "surface3DChart",
];

fn chart_json(b: &Book, root: &Node, theme: &HashMap<String, String>, auto: &AutoColors) -> Map<String, J> {
    let mut out = Map::new();
    let Some(chart) = root.child("chart") else { return out };
    if let Some(t) = chart.child("title") {
        let text = t.child("tx").and_then(|tx| text_of(b, tx));
        out.insert("title".into(), text.map(J::String).unwrap_or(J::Bool(true)));
    }
    out.insert("autoTitleDeleted".into(), J::Bool(chart.flag("autoTitleDeleted").unwrap_or(false)));
    out.insert("legend".into(), chart.child("legend").map(|l| J::String(l.val("legendPos").unwrap_or("r").to_string())).unwrap_or(J::Null));
    let Some(plot) = chart.child("plotArea") else { return out };
    out.insert("dataTable".into(), J::Bool(plot.child("dTable").is_some()));
    let mut groups = Vec::new();
    for g in plot.children.iter().filter(|c| GROUPS.contains(&c.name.as_str())) {
        let line = matches!(g.name.as_str(), "lineChart" | "line3DChart" | "scatterChart" | "radarChart" | "stockChart");
        let mut series = Vec::new();
        for s in g.all("ser") {
            let mut o = Map::new();
            o.insert("name".into(), s.child("tx").and_then(|tx| text_of(b, tx)).map(J::String).unwrap_or(J::Null));
            // Lines take their color from the line; markers-only scatter from the marker.
            let marker_none = s.path(&["marker", "symbol"]).and_then(|m| m.attr("val")) == Some("none");
            let (mut color, no_fill) = shape(s.child("spPr"), line, theme);
            if color.is_none() && line {
                color = shape(s.path(&["marker", "spPr"]), false, theme).0;
            }
            // No color of its own: the automatic color of its index (a line
            // without its line still has its markers in that color).
            if color.is_none() && (!no_fill || line) {
                let idx = s.val("idx").and_then(|v| v.parse::<usize>().ok()).unwrap_or(series.len());
                color = auto.nth(idx);
            }
            if let Some(c) = color {
                o.insert("color".into(), J::String(c));
            }
            o.insert("noFill".into(), J::Bool(no_fill));
            o.insert("marker".into(), J::Bool(!marker_none));
            o.insert("smooth".into(), J::Bool(s.flag("smooth").unwrap_or(false)));
            o.insert("trendline".into(), J::Bool(s.child("trendline").is_some()));
            o.insert("errBars".into(), J::Bool(s.child("errBars").is_some()));
            // Colors of single points (pies).
            let points: Vec<J> = s
                .all("dPt")
                .filter_map(|p| {
                    let idx = p.val("idx")?.parse::<u32>().ok()?;
                    let (c, _) = shape(p.child("spPr"), false, theme);
                    Some(json!({ "idx": idx, "color": c }))
                })
                .collect();
            if !points.is_empty() {
                o.insert("points".into(), J::Array(points));
            }
            for (k, name) in [("cat", "cat"), ("val", "val"), ("xVal", "xVal"), ("yVal", "yVal"), ("bubbleSize", "bubbleSize")] {
                if let Some(n) = s.child(name) {
                    o.insert(k.into(), source(b, n));
                }
            }
            series.push(J::Object(o));
        }
        let mut o = Map::new();
        o.insert("kind".into(), J::String(g.name.clone()));
        for k in ["barDir", "grouping", "scatterStyle", "holeSize", "radarStyle"] {
            if let Some(v) = g.val(k) {
                o.insert(k.into(), J::String(v.to_string()));
            }
        }
        o.insert("varyColors".into(), J::Bool(g.flag("varyColors").unwrap_or(false)));
        let marker = g.flag("marker");
        if let Some(m) = marker {
            o.insert("marker".into(), J::Bool(m));
        }
        o.insert("axes".into(), J::Array(g.all("axId").filter_map(|a| a.attr("val")).map(|v| J::String(v.into())).collect()));
        o.insert("series".into(), J::Array(series));
        groups.push(J::Object(o));
    }
    out.insert("groups".into(), J::Array(groups));
    // The automatic colors of points (pies whose slices have no color of their own).
    out.insert("autoColors".into(), J::Array((0..64).filter_map(|i| auto.nth(i)).map(J::String).collect()));
    let mut axes = Vec::new();
    for a in plot.children.iter().filter(|c| matches!(c.name.as_str(), "catAx" | "dateAx" | "valAx" | "serAx")) {
        let mut o = Map::new();
        o.insert("kind".into(), J::String(a.name.clone()));
        o.insert("id".into(), J::String(a.val("axId").unwrap_or("").into()));
        o.insert("pos".into(), J::String(a.val("axPos").unwrap_or("").into()));
        o.insert("deleted".into(), J::Bool(a.flag("delete").unwrap_or(false)));
        if let Some(t) = a.child("title") {
            o.insert("title".into(), t.child("tx").and_then(|tx| text_of(b, tx)).map(J::String).unwrap_or(J::Null));
        }
        if let Some(nf) = a.child("numFmt") {
            o.insert("format".into(), J::String(nf.attr("formatCode").unwrap_or("").into()));
            o.insert("sourceLinked".into(), J::Bool(matches!(nf.attr("sourceLinked"), Some("1" | "true"))));
        }
        if let Some(sc) = a.child("scaling") {
            o.insert("reversed".into(), J::Bool(sc.val("orientation") == Some("maxMin")));
            for k in ["min", "max", "logBase"] {
                if let Some(v) = sc.val(k).and_then(|v| v.parse::<f64>().ok()) {
                    o.insert(k.into(), json!(v));
                }
            }
        }
        axes.push(J::Object(o));
    }
    out.insert("axes".into(), J::Array(axes));
    out
}

/// The kind of a chartEx chart (waterfall, funnel, treemap, ...).
fn chart_ex_kind(root: &Node) -> String {
    root.find("series").and_then(|s| s.attr("layoutId")).unwrap_or("chartEx").to_string()
}

/// "E2:L18" for the cells an anchor covers.
fn anchor_range(a: &Node) -> J {
    let at = |n: &str| -> Option<(u32, u32)> {
        let m = a.child(n)?;
        let col = m.child("col")?.text.trim().parse::<u32>().ok()?;
        let row = m.child("row")?.text.trim().parse::<u32>().ok()?;
        Some((col + 1, row + 1))
    };
    match (at("from"), at("to")) {
        (Some(f), Some(t)) => J::String(format!("{}:{}", super::xlsx::cell(f.0, f.1), super::xlsx::cell(t.0, t.1))),
        (Some(f), None) => J::String(super::xlsx::cell(f.0, f.1)),
        _ => J::Null,
    }
}

/// Calls f with the anchor, the graphic frame, the relationship type and
/// the part of each chart on a sheet, in the order of its drawing.
fn each_chart(parts: &ChartParts, sheet: &str, mut f: impl FnMut(&Node, &Node, &str, &str)) {
    if parts.files.is_empty() {
        return;
    }
    let Some(sheet_part) = parts.sheet_part(sheet) else { return };
    let mut drawings: Vec<String> =
        parts.rels(&sheet_part).into_values().filter(|(t, _)| t.ends_with("/drawing")).map(|(_, p)| p).collect();
    drawings.sort();
    for d in drawings {
        let Some(drawing) = parts.xml(&d) else { continue };
        let rels = parts.rels(&d);
        for anchor in drawing.children.iter().filter(|c| c.name.ends_with("Anchor")) {
            let Some(frame) = anchor.find("graphicFrame") else { continue };
            let Some(data) = frame.find("graphicData") else { continue };
            let Some(id) = data.children.iter().find(|c| c.name == "chart").and_then(|c| c.attr("id")) else { continue };
            let Some((kind, part)) = rels.get(id) else { continue };
            f(anchor, frame, kind, part);
        }
    }
}

/// The charts on a sheet, in the order of its drawing.
pub fn sheet_charts(b: &Book, sheet: &str) -> Vec<J> {
    let mut out = Vec::new();
    let Some(parts) = &b.charts else { return out };
    let theme = parts.theme();
    each_chart(parts, sheet, |anchor, frame, kind, part| {
        let Some(root) = parts.xml(part) else { return };
        let mut o = if kind.ends_with("/chartEx") {
            let mut m = Map::new();
            m.insert("chartEx".into(), J::String(chart_ex_kind(&root)));
            m
        } else {
            chart_json(b, &root, &theme, &AutoColors::new(parts, part, &theme))
        };
        let name = frame.path(&["nvGraphicFramePr", "cNvPr"]).and_then(|c| c.attr("name")).unwrap_or("");
        o.insert("name".into(), J::String(name.to_string()));
        o.insert("range".into(), anchor_range(anchor));
        out.push(J::Object(o));
    });
    out
}

/// Where a drawing object is on its sheet: the cell markers (zero-based
/// column and row, offsets in EMU) and/or the size and position in EMU.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Anchor {
    pub from: Option<(u32, i64, u32, i64)>,
    pub to: Option<(u32, i64, u32, i64)>,
    pub ext: Option<(i64, i64)>,
    pub pos: Option<(i64, i64)>,
}

/// The anchor of a drawing (twoCellAnchor, oneCellAnchor, absoluteAnchor).
pub(super) fn anchor_of(a: &Node) -> Anchor {
    let marker = |n: &str| -> Option<(u32, i64, u32, i64)> {
        let m = a.child(n)?;
        let num = |k: &str| m.child(k).and_then(|c| c.text.trim().parse::<i64>().ok());
        Some((num("col")? as u32, num("colOff").unwrap_or(0), num("row")? as u32, num("rowOff").unwrap_or(0)))
    };
    let pair = |n: &str, x: &str, y: &str| -> Option<(i64, i64)> {
        let e = a.child(n)?;
        Some((e.attr(x)?.parse().ok()?, e.attr(y)?.parse().ok()?))
    };
    Anchor { from: marker("from"), to: marker("to"), ext: pair("ext", "cx", "cy"), pos: pair("pos", "x", "y") }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_modifiers() {
        let m = |v: &[(&str, f64)]| v.iter().map(|(k, x)| (k.to_string(), *x)).collect::<Vec<_>>();
        // As Excel shows them (Office 2013+ theme): accent1 darker 40% / lighter 40%, accent2 darker 25%.
        assert_eq!(modified("4472C4", &m(&[("lumMod", 0.6)])).unwrap(), "#264478");
        assert_eq!(modified("4472C4", &m(&[("lumMod", 0.6), ("lumOff", 0.4)])).unwrap(), "#8FAADC");
        assert_eq!(modified("ED7D31", &m(&[("lumMod", 0.75)])).unwrap(), "#C55A11");
        assert_eq!(modified("70AD47", &[]).unwrap(), "#70AD47");
    }

    #[test]
    fn default_auto_colors() {
        let a = AutoColors::new(&ChartParts::default(), "xl/charts/chart1.xml", &HashMap::new());
        let got: Vec<String> = [0, 1, 5, 6, 12].iter().map(|i| a.nth(*i).unwrap()).collect();
        assert_eq!(got, ["#4472C4", "#ED7D31", "#70AD47", "#264478", "#698ED0"]);
    }
}
