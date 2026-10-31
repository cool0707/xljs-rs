//! The print layout of worksheets as HTML: the pages Excel would print
//! (paper, margins, scale or "fit to", print areas and titles, page breaks,
//! headers and footers), with the cells' values, formats, merged cells,
//! pictures and the places of the charts. The page web/print.html draws the
//! charts into it and WebView2 turns it into a PDF (book.toPdf).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{Cursor, Read};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use umya_spreadsheet::structs::EnumTrait;
use umya_spreadsheet::structs::drawing::Theme;
use umya_spreadsheet::{Cell, CellRawValue, Color, Style, Worksheet};

use super::xlsx::{Book, format_code, from_serial, style_is_date};
use super::xlsx_chart::{Anchor, parse, resolve};
use crate::{Result, err};

/// EMU per point.
const EMU_PT: f64 = 12700.0;
const PT_MM: f64 = 72.0 / 25.4;

/// What the page setup of a sheet says that umya-spreadsheet does not keep
/// (or keeps without telling a missing attribute from 0).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawPrint {
    /// fitToWidth / fitToHeight as written (absent = 1, 0 = automatic).
    pub fit_width: Option<u32>,
    pub fit_height: Option<u32>,
    /// pageOrder="overThenDown".
    pub over_then_down: bool,
    /// printOptions gridLines.
    pub grid_lines: bool,
    /// The sheet is hidden (state="hidden" / "veryHidden" in workbook.xml).
    pub hidden: bool,
}

/// Reads the page setup of every sheet of an xlsx file (see RawPrint).
pub fn collect_raw(data: &[u8]) -> HashMap<String, RawPrint> {
    let mut out = HashMap::new();
    let Ok(mut zr) = zip::ZipArchive::new(Cursor::new(data)) else { return out };
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let mut f = zr.by_name(name).ok()?;
        let mut b = Vec::new();
        f.read_to_end(&mut b).ok()?;
        Some(b)
    };
    let Some(wb) = read("xl/workbook.xml").and_then(|b| parse(&b)) else { return out };
    let rels: HashMap<String, String> = read("xl/_rels/workbook.xml.rels")
        .and_then(|b| parse(&b))
        .map(|r| r.all("Relationship").filter_map(|x| Some((x.attr("Id")?.to_string(), resolve("xl", x.attr("Target")?)))).collect())
        .unwrap_or_default();
    let Some(sheets) = wb.child("sheets") else { return out };
    for s in sheets.all("sheet") {
        let (Some(name), Some(id)) = (s.attr("name"), s.attr("id")) else { continue };
        let Some(part) = rels.get(id) else { continue };
        let Some(xml) = read(part) else { continue };
        let mut r = raw_print(&xml);
        r.hidden = matches!(s.attr("state"), Some("hidden" | "veryHidden"));
        out.insert(name.to_string(), r);
    }
    out
}

/// The page setup of a sheet's XML. Only the elements before sheetData and
/// after it matter, so the cells are skipped with a plain search.
fn raw_print(xml: &[u8]) -> RawPrint {
    let s = String::from_utf8_lossy(xml);
    let attr = |tag: &str, name: &str| -> Option<String> {
        let i = s.find(&format!("<{tag} "))?;
        let rest = &s[i..];
        let end = rest.find('>')?;
        let el = &rest[..end];
        let k = el.find(&format!(" {name}=\""))? + name.len() + 3;
        Some(el[k..].split('"').next()?.to_string())
    };
    let on = |v: Option<String>| matches!(v.as_deref(), Some("1" | "true"));
    RawPrint {
        fit_width: attr("pageSetup", "fitToWidth").and_then(|v| v.parse().ok()),
        fit_height: attr("pageSetup", "fitToHeight").and_then(|v| v.parse().ok()),
        over_then_down: attr("pageSetup", "pageOrder").as_deref() == Some("overThenDown"),
        grid_lines: on(attr("printOptions", "gridLines")),
        hidden: false,
    }
}

/// Margins in millimeters (None = the sheet's).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct Margins {
    pub top: Option<f64>,
    pub right: Option<f64>,
    pub bottom: Option<f64>,
    pub left: Option<f64>,
}

/// What book.toPdf / toHtml may change of the sheets' page setup.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintOptions {
    /// The sheets to print, in order (empty = every visible sheet).
    pub sheets: Vec<String>,
    /// "A4", "A3", "B5", "Letter", ... (None = the sheet's).
    pub paper: Option<String>,
    pub landscape: Option<bool>,
    /// Fit to this many pages wide / tall (0 = automatic). Either one set
    /// means "fit to" whatever the sheet says.
    pub fit_to_width: Option<u32>,
    pub fit_to_height: Option<u32>,
    /// The scale in percent (instead of "fit to").
    pub scale: Option<f64>,
    pub margin: Option<Margins>,
    pub grid_lines: Option<bool>,
    pub center_horizontally: Option<bool>,
    /// Draw the charts (default true).
    pub charts: Option<bool>,
    pub title: String,
}

/// A chart placed in the HTML: the index-th of sheet.charts.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChartRef {
    pub sheet: String,
    pub index: usize,
}

/// The pages of a workbook as an HTML document.
#[derive(Debug, Clone)]
pub struct PrintOut {
    pub html: String,
    /// The charts that `<div class="xljs-print-chart" data-chart="i">`
    /// stands for (several divs may show the same chart, cut by pages).
    pub charts: Vec<ChartRef>,
    pub pages: usize,
}

/// The size of a paper in millimeters (portrait) by Excel's paperSize code
/// or by name.
pub fn paper_size(code: u32) -> (f64, f64) {
    match code {
        1 | 2 => (215.9, 279.4),
        3 => (279.4, 431.8),
        4 => (431.8, 279.4),
        5 => (215.9, 355.6),
        6 => (139.7, 215.9),
        7 => (184.15, 266.7),
        8 => (297.0, 420.0),
        11 => (148.0, 210.0),
        12 => (257.0, 364.0),
        13 => (182.0, 257.0),
        14 => (215.9, 330.2),
        20 => (104.8, 241.3),
        27 => (110.0, 220.0),
        28 => (162.0, 229.0),
        34 => (176.0, 250.0),
        43 => (100.0, 148.0),
        66 => (420.0, 594.0),
        70 => (105.0, 148.0),
        _ => (210.0, 297.0),
    }
}

/// The paperSize code of a paper name ("A4", "b5", "Letter"); None if unknown.
pub fn paper_code(name: &str) -> Option<u32> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "letter" => 1,
        "tabloid" => 3,
        "ledger" => 4,
        "legal" => 5,
        "statement" => 6,
        "executive" => 7,
        "a2" => 66,
        "a3" => 8,
        "a4" => 9,
        "a5" => 11,
        "a6" => 70,
        "b4" => 12,
        "b5" => 13,
        "folio" => 14,
        "postcard" | "hagaki" => 43,
        _ => return None,
    })
}

pub(super) fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

/// A number for CSS (at most 3 decimals).
pub(super) fn n(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// "#RRGGBB" of a color (theme colors from the workbook's theme); None for
/// automatic or missing colors.
fn css_color(c: &Color, theme: &Theme) -> Option<String> {
    if c.automatic() {
        return None;
    }
    let s = c.argb_with_theme(theme);
    if s.is_empty() || s == "00000000" || s.len() < 6 {
        return None;
    }
    Some(format!("#{}", &s[s.len() - 6..]))
}

/// A font-family list for an Excel font name (Japanese names also by their
/// English names, which every Windows knows).
pub(super) fn font_family(name: &str) -> String {
    let alias = match name {
        "游ゴシック" => "Yu Gothic",
        "游ゴシック Light" => "Yu Gothic Light",
        "游ゴシック Medium" => "Yu Gothic Medium",
        "游明朝" => "Yu Mincho",
        "ＭＳ Ｐゴシック" => "MS PGothic",
        "ＭＳ ゴシック" => "MS Gothic",
        "ＭＳ Ｐ明朝" => "MS PMincho",
        "ＭＳ 明朝" => "MS Mincho",
        "メイリオ" => "Meiryo",
        "BIZ UDPゴシック" => "BIZ UDPGothic",
        "BIZ UDゴシック" => "BIZ UDGothic",
        "BIZ UDP明朝" => "BIZ UDPMincho",
        "BIZ UD明朝" => "BIZ UDMincho",
        _ => "",
    };
    let q = |s: &str| format!("\"{}\"", s.replace(['\\', '"'], ""));
    let mut out = q(name);
    if !alias.is_empty() {
        out.push(',');
        out.push_str(&q(alias));
    }
    if name.contains("明朝") || name.contains("Mincho") {
        out.push_str(",\"Yu Mincho\",serif");
    } else {
        out.push_str(",\"Yu Gothic\",Meiryo,sans-serif");
    }
    out
}

fn japanese_font(name: &str) -> bool {
    !name.is_ascii() || name.starts_with("Yu ") || name.starts_with("MS ") || name.starts_with("Meiryo") || name.starts_with("BIZ ")
}

/// The CSS of a border side (Excel draws thin as one pixel).
fn border_css(style: &str, color: &str) -> Option<String> {
    let (w, s) = match style {
        "thin" => (0.75, "solid"),
        "hair" => (0.5, "solid"),
        "dotted" => (0.75, "dotted"),
        "dashed" | "dashDot" | "dashDotDot" => (0.75, "dashed"),
        "medium" => (1.5, "solid"),
        "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" | "slantDashDot" => (1.5, "dashed"),
        "thick" => (2.25, "solid"),
        "double" => (2.25, "double"),
        _ => return None,
    };
    Some(format!("{}pt {s} {color}", n(w)))
}

/// The color of a number format's section for a value ([Red], [赤] ...).
fn format_color(code: &str, v: Option<f64>) -> Option<&'static str> {
    let mut secs = vec![String::new()];
    let mut q = false;
    for c in code.chars() {
        match c {
            '"' => {
                q = !q;
                secs.last_mut().unwrap().push(c);
            }
            ';' if !q => secs.push(String::new()),
            _ => secs.last_mut().unwrap().push(c),
        }
    }
    let sec = match v {
        Some(f) if f < 0.0 && secs.len() >= 2 => &secs[1],
        Some(f) if f == 0.0 && secs.len() >= 3 => &secs[2],
        Some(_) => &secs[0],
        None if secs.len() >= 4 => &secs[3],
        None => return None,
    };
    let lower = sec.to_lowercase();
    for (k, c) in [
        ("[red]", "#FF0000"),
        ("[赤]", "#FF0000"),
        ("[blue]", "#0000FF"),
        ("[青]", "#0000FF"),
        ("[green]", "#00FF00"),
        ("[緑]", "#00FF00"),
        ("[black]", "#000000"),
        ("[黒]", "#000000"),
        ("[white]", "#FFFFFF"),
        ("[白]", "#FFFFFF"),
        ("[yellow]", "#FFFF00"),
        ("[黄]", "#FFFF00"),
        ("[magenta]", "#FF00FF"),
        ("[紫]", "#FF00FF"),
        ("[cyan]", "#00FFFF"),
        ("[水]", "#00FFFF"),
    ] {
        if lower.contains(k) {
            return Some(c);
        }
    }
    None
}

/// A cell's value for display.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Empty,
    Num(f64, String),
    Text(String),
    Bool(bool),
    Error(String),
}

/// A rectangle of cells (1-based, inclusive): c1, r1, c2, r2.
type Area = (u32, u32, u32, u32);

/// A segment of rows or columns that a page shows.
type Seg = (u32, u32);

/// "A1:D10", "$A:$C", "$1:$3" (sheet prefix and $ removed) as an area;
/// whole columns or rows run to the given limits.
fn parse_area(s: &str, max_col: u32, max_row: u32) -> Option<Area> {
    let s = s.rsplit('!').next()?.replace('$', "");
    let (a, b) = s.split_once(':').unwrap_or((&s, &s));
    let part = |p: &str| -> Option<(Option<u32>, Option<u32>)> {
        let p = p.trim();
        let letters: String = p.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let digits = &p[letters.len()..];
        let col = if letters.is_empty() { None } else { Some(super::xlsx::column_number(&letters.to_uppercase()).ok()?) };
        let row = if digits.is_empty() { None } else { Some(digits.parse().ok()?) };
        if col.is_none() && row.is_none() {
            return None;
        }
        Some((col, row))
    };
    let (ca, ra) = part(a)?;
    let (cb, rb) = part(b)?;
    let c1 = ca.unwrap_or(1);
    let c2 = cb.unwrap_or(if ca.is_none() { max_col } else { c1 });
    let r1 = ra.unwrap_or(1);
    let r2 = rb.unwrap_or(if ra.is_none() { max_row } else { r1 });
    Some((c1.min(c2), r1.min(r2), c1.max(c2), r1.max(r2)))
}

/// Splits a defined name's address at the commas outside quotes.
fn split_refs(s: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut q = false;
    for c in s.chars() {
        match c {
            '\'' => {
                q = !q;
                out.last_mut().unwrap().push(c);
            }
            ',' if !q => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// How a sheet is laid out on paper.
struct Setup {
    paper: (f64, f64), // pt, oriented
    margins: [f64; 4], // top right bottom left, pt
    header: f64,       // pt from the top edge
    footer: f64,       // pt from the bottom edge
    scale: Option<f64>,
    fit: Option<(u32, u32)>,
    over_then_down: bool,
    grid: bool,
    center_h: bool,
    center_v: bool,
}

struct Sheet<'a> {
    b: &'a Book,
    ws: &'a Worksheet,
    name: String,
    theme: &'a Theme,
    cells: HashMap<(u32, u32), &'a Cell>,
    merges: Vec<Area>,
    merge_at: HashMap<(u32, u32), usize>,
    mdw: f64,
    default_w: f64,
    default_h: f64,
    col_w: HashMap<u32, f64>,
    row_h: HashMap<u32, f64>,
    x_cache: std::cell::RefCell<Vec<f64>>,
    y_cache: std::cell::RefCell<Vec<f64>>,
    font: (String, f64),
}

impl<'a> Sheet<'a> {
    fn new(b: &'a Book, name: &str) -> Result<Sheet<'a>> {
        let ws = b.sheet(name)?;
        let font = b.default_font.clone();
        let mdw = ((if japanese_font(&font.0) { 8.0 } else { 7.0 }) * font.1 / 11.0).round().max(1.0);
        let px = |w: f64| ((256.0 * w + (128.0 / mdw).trunc()) / 256.0 * mdw).trunc();
        let fmt = ws.sheet_format_properties();
        let default_w = if fmt.default_column_width() > 0.0 {
            px(fmt.default_column_width())
        } else {
            let base = if fmt.base_column_width() > 0 { fmt.base_column_width() as f64 } else { 8.0 };
            ((base * mdw + 5.0) / 8.0).ceil() * 8.0
        } * 0.75;
        // Excel sizes rows by the Normal font unless the sheet says otherwise
        // (customHeight); new workbooks of umya-spreadsheet say 13.5 whatever the font.
        let by_font = if font.0.contains("游") || font.0.starts_with("Yu Gothic") {
            Some(18.75 / 11.0)
        } else if font.0.starts_with("ＭＳ") || font.0.starts_with("MS ") {
            Some(13.5 / 11.0)
        } else if font.0 == "Calibri" {
            Some(15.0 / 11.0)
        } else {
            None
        };
        let default_h = match by_font {
            Some(k) if !fmt.custom_height() => ((font.1 * k) / 0.75).round() * 0.75,
            _ if fmt.default_row_height() > 0.0 => fmt.default_row_height(),
            Some(k) => ((font.1 * k) / 0.75).round() * 0.75,
            None => ((font.1 * 15.0 / 11.0) / 0.75).round() * 0.75,
        };
        let mut col_w = HashMap::new();
        for c in ws.column_dimensions() {
            let w = if c.hidden() { 0.0 } else { px(c.width()) * 0.75 };
            col_w.insert(c.col_num(), w);
        }
        let mut row_h = HashMap::new();
        for r in ws.row_dimensions() {
            if r.hidden() {
                row_h.insert(r.row_num(), 0.0);
            } else if r.height() > 0.0 {
                row_h.insert(r.row_num(), r.height());
            }
        }
        let mut cells = HashMap::new();
        for c in ws.cells() {
            let co = c.coordinate();
            cells.insert((co.col_num(), co.row_num()), c);
        }
        let mut merges = Vec::new();
        let mut merge_at = HashMap::new();
        for m in ws.merge_cells() {
            let Ok(a) = super::xlsx_style::parse_range(&m.range()) else { continue };
            for r in a.1..=a.3 {
                for c in a.0..=a.2 {
                    merge_at.insert((c, r), merges.len());
                }
            }
            merges.push(a);
        }
        Ok(Sheet {
            b,
            ws,
            name: name.to_string(),
            theme: b.wb.theme(),
            cells,
            merges,
            merge_at,
            mdw,
            default_w,
            default_h,
            col_w,
            row_h,
            x_cache: Default::default(),
            y_cache: Default::default(),
            font,
        })
    }

    fn w(&self, c: u32) -> f64 {
        *self.col_w.get(&c).unwrap_or(&self.default_w)
    }

    fn h(&self, r: u32) -> f64 {
        *self.row_h.get(&r).unwrap_or(&self.default_h)
    }

    /// The left edge of the zero-based column c (pt from the sheet's left).
    fn x0(&self, c: u32) -> f64 {
        let mut v = self.x_cache.borrow_mut();
        if v.is_empty() {
            v.push(0.0);
        }
        while v.len() <= c as usize {
            let i = v.len() as u32; // the 1-based column before edge i
            let last = *v.last().unwrap();
            v.push(last + self.w(i));
        }
        v[c as usize]
    }

    fn y0(&self, r: u32) -> f64 {
        let mut v = self.y_cache.borrow_mut();
        if v.is_empty() {
            v.push(0.0);
        }
        while v.len() <= r as usize {
            let i = v.len() as u32;
            let last = *v.last().unwrap();
            v.push(last + self.h(i));
        }
        v[r as usize]
    }

    fn val(&self, col: u32, row: u32) -> Val {
        let Some(c) = self.cells.get(&(col, row)) else { return Val::Empty };
        let num = |f: f64| -> Val {
            let nf = c.style().number_format();
            let code = format_code(nf);
            if style_is_date(nf)
                && let Some(t) = from_serial(f, self.b.date1904)
            {
                return Val::Num(f, super::numfmt::format_date(t, &code));
            }
            Val::Num(f, super::numfmt::format_number(f, &code))
        };
        match c.raw_value() {
            CellRawValue::Empty => {
                if c.is_formula() {
                    // A formula without a cached result (written by a script).
                    let r = super::xlsx::cell(col, row);
                    return match super::xlsx_calc::calc_cell(self.b, &self.name, &r) {
                        Ok(s) if s.is_empty() => Val::Empty,
                        Ok(s) if s.starts_with('#') => Val::Error(s),
                        Ok(s) => match s.parse::<f64>() {
                            Ok(f) => num(f),
                            Err(_) if s == "TRUE" || s == "FALSE" => Val::Bool(s == "TRUE"),
                            Err(_) => Val::Text(s),
                        },
                        Err(_) => Val::Empty,
                    };
                }
                Val::Empty
            }
            CellRawValue::Numeric(f) => num(*f),
            CellRawValue::Bool(v) => Val::Bool(*v),
            CellRawValue::Error(e) => Val::Error(e.to_string()),
            other => {
                let s = other.to_string();
                if s.is_empty() { Val::Empty } else { Val::Text(s) }
            }
        }
    }

    /// Whether a cell shows something on paper (a value, a fill or a border).
    fn printed(&self, c: &Cell) -> bool {
        let co = c.coordinate();
        if self.val(co.col_num(), co.row_num()) != Val::Empty {
            return true;
        }
        let st = c.style();
        if self.fill(st).is_some() {
            return true;
        }
        if let Some(b) = st.borders() {
            for side in [b.left(), b.right(), b.top(), b.bottom()] {
                let s = side.border_style();
                if !s.is_empty() && s != "none" {
                    return true;
                }
            }
        }
        false
    }

    fn fill(&self, st: &Style) -> Option<String> {
        let pf = st.fill()?.pattern_fill()?;
        let t = pf.pattern_type().value_string();
        if t == "none" || t.is_empty() || t == "gray125" {
            return None;
        }
        let c = pf.foreground_color().and_then(|c| css_color(c, self.theme));
        if t == "solid" {
            return c;
        }
        c.or_else(|| pf.background_color().and_then(|c| css_color(c, self.theme)))
    }

    fn defined(&self, name: &str) -> Vec<String> {
        let full = format!("_xlnm.{name}");
        for d in self.ws.defined_names() {
            if d.name() == full || d.name() == name {
                return split_refs(&d.address());
            }
        }
        // Names of the workbook that point at this sheet.
        for d in self.b.wb.defined_names() {
            if (d.name() == full || d.name() == name) && d.has_local_sheet_id() {
                let idx = d.local_sheet_id() as usize;
                if self.b.wb.sheet_collection().get(idx).is_some_and(|s| s.name() == self.name) {
                    return split_refs(&d.address());
                }
            }
        }
        Vec::new()
    }

    /// The print areas, or else the used range (cells with a value, fill or
    /// border, merged cells, pictures and charts).
    fn areas(&self, anchors: &[Anchor]) -> Vec<Area> {
        let refs = self.defined("Print_Area");
        let areas: Vec<Area> = refs.iter().filter_map(|r| parse_area(r, 16384, 1048576)).collect();
        if !areas.is_empty() {
            return areas;
        }
        let mut a: Option<Area> = None;
        let mut add = |c: u32, r: u32| {
            a = Some(match a {
                None => (c, r, c, r),
                Some((c1, r1, c2, r2)) => (c1.min(c), r1.min(r), c2.max(c), r2.max(r)),
            });
        };
        for c in self.cells.values() {
            if self.printed(c) {
                let co = c.coordinate();
                add(co.col_num(), co.row_num());
            }
        }
        for m in &self.merges {
            if self.val(m.0, m.1) != Val::Empty {
                add(m.0, m.1);
                add(m.2, m.3);
            }
        }
        for an in anchors {
            if let Some((c1, r1, c2, r2)) = self.anchor_cells(an) {
                add(c1, r1);
                add(c2, r2);
            }
        }
        a.into_iter().collect()
    }

    /// The cells an anchor covers (1-based).
    fn anchor_cells(&self, an: &Anchor) -> Option<Area> {
        let (x, y, w, h) = self.anchor_rect(an)?;
        let col_at = |x: f64| -> u32 {
            let mut c = 1;
            while self.x0(c) <= x && c < 16384 {
                c += 1;
            }
            c
        };
        let row_at = |y: f64| -> u32 {
            let mut r = 1;
            while self.y0(r) <= y && r < 1048576 {
                r += 1;
            }
            r
        };
        let (c1, r1) = (col_at(x), row_at(y));
        let (c2, r2) = (col_at((x + w - 0.01).max(x)), row_at((y + h - 0.01).max(y)));
        Some((c1, r1, c2, r2))
    }

    /// The rectangle of an anchor in pt from the sheet's top left.
    fn anchor_rect(&self, an: &Anchor) -> Option<(f64, f64, f64, f64)> {
        let at = |m: (u32, i64, u32, i64)| (self.x0(m.0) + m.1 as f64 / EMU_PT, self.y0(m.2) + m.3 as f64 / EMU_PT);
        let (x, y) = match (an.from, an.pos) {
            (Some(f), _) => at(f),
            (None, Some((x, y))) => (x as f64 / EMU_PT, y as f64 / EMU_PT),
            _ => return None,
        };
        let (w, h) = match (an.to, an.ext) {
            (Some(t), _) => {
                let (x2, y2) = at(t);
                (x2 - x, y2 - y)
            }
            (None, Some((cx, cy))) => (cx as f64 / EMU_PT, cy as f64 / EMU_PT),
            _ => return None,
        };
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some((x, y, w, h))
    }

    /// The print titles: rows and columns.
    fn titles(&self) -> (Option<Seg>, Option<Seg>) {
        let (mut rows, mut cols) = (None, None);
        for r in self.defined("Print_Titles") {
            let Some(a) = parse_area(&r, 16384, 1048576) else { continue };
            if a.0 == 1 && a.2 == 16384 {
                rows = Some((a.1, a.3));
            } else if a.1 == 1 && a.3 == 1048576 {
                cols = Some((a.0, a.2));
            }
        }
        (rows, cols)
    }

    fn breaks(&self, rows: bool) -> HashSet<u32> {
        let list = if rows { self.ws.row_breaks().break_list() } else { self.ws.column_breaks().break_list() };
        list.iter().map(|b| b.id() + 1).collect()
    }
}

/// Splits first..=last into pages of at most avail pt (titles of title pt
/// come before every segment that starts after title_last). A break at i
/// starts a new page at i.
fn split(first: u32, last: u32, size: &dyn Fn(u32) -> f64, avail: f64, title: f64, title_last: u32, breaks: &HashSet<u32>) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut start = first;
    while start <= last {
        // Skip hidden rows/columns at the start.
        while start <= last && size(start) <= 0.0 {
            start += 1;
        }
        if start > last {
            break;
        }
        let room = avail - if title > 0.0 && start > title_last { title } else { 0.0 };
        let mut used = 0.0;
        let mut end = start;
        let mut i = start;
        while i <= last {
            if i > start && breaks.contains(&i) {
                break;
            }
            let s = size(i);
            if i > start && used + s > room + 0.01 {
                break;
            }
            used += s;
            end = i;
            i += 1;
        }
        out.push((start, end));
        start = end + 1;
    }
    out
}

/// Builds the style of the header or footer text of one side ("&L", "&C",
/// "&R" sections): CSS content and the font properties.
fn hf_content(text: &str, sheet: &str, file: &str, dir: &str, now: &(String, String)) -> (String, String) {
    let mut parts: Vec<String> = Vec::new();
    let mut lit = String::new();
    let mut css = String::new();
    let flush = |lit: &mut String, parts: &mut Vec<String>| {
        if !lit.is_empty() {
            let s = lit.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\A ").replace('\r', "");
            parts.push(format!("\"{s}\""));
            lit.clear();
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '&' || i + 1 >= chars.len() {
            lit.push(c);
            i += 1;
            continue;
        }
        let k = chars[i + 1];
        i += 2;
        match k {
            '&' => lit.push('&'),
            'P' => {
                flush(&mut lit, &mut parts);
                parts.push("counter(page)".into());
            }
            'N' => {
                flush(&mut lit, &mut parts);
                parts.push("counter(pages)".into());
            }
            'D' => lit.push_str(&now.0),
            'T' => lit.push_str(&now.1),
            'A' => lit.push_str(sheet),
            'F' => lit.push_str(file),
            'Z' => lit.push_str(dir),
            'B' => css.push_str("font-weight:bold;"),
            'I' => css.push_str("font-style:italic;"),
            'U' | 'E' => css.push_str("text-decoration:underline;"),
            'S' => css.push_str("text-decoration:line-through;"),
            '"' => {
                // &"Font,Style"
                let mut f = String::new();
                while i < chars.len() && chars[i] != '"' {
                    f.push(chars[i]);
                    i += 1;
                }
                i += 1;
                let (name, style) = f.split_once(',').unwrap_or((&f, ""));
                if !name.is_empty() && name != "-" {
                    let _ = write!(css, "font-family:{};", font_family(name));
                }
                let st = style.to_lowercase();
                if st.contains("bold") || st.contains("太字") {
                    css.push_str("font-weight:bold;");
                }
                if st.contains("italic") || st.contains("斜体") {
                    css.push_str("font-style:italic;");
                }
            }
            'K' => {
                let hex: String = chars[i..].iter().take(6).collect();
                if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    let _ = write!(css, "color:#{hex};");
                    i += 6;
                }
            }
            d if d.is_ascii_digit() => {
                let mut num = d.to_string();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num.push(chars[i]);
                    i += 1;
                }
                let _ = write!(css, "font-size:{num}pt;");
            }
            _ => {} // &G (picture), &X, &Y, ...
        }
    }
    flush(&mut lit, &mut parts);
    (parts.join(" "), css)
}

/// The &L / &C / &R sections of a header or footer.
fn hf_sections(s: &str) -> [String; 3] {
    let mut out = [String::new(), String::new(), String::new()];
    let mut cur = 1; // text before any code is centered
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' && i + 1 < chars.len() {
            match chars[i + 1] {
                'L' => {
                    cur = 0;
                    i += 2;
                    continue;
                }
                'C' => {
                    cur = 1;
                    i += 2;
                    continue;
                }
                'R' => {
                    cur = 2;
                    i += 2;
                    continue;
                }
                c => {
                    out[cur].push('&');
                    out[cur].push(c);
                    i += 2;
                    continue;
                }
            }
        }
        out[cur].push(chars[i]);
        i += 1;
    }
    out
}

/// Collects the CSS rules of cells, one class per distinct rule.
#[derive(Default)]
struct Classes {
    by_rule: HashMap<String, usize>,
    css: String,
}

impl Classes {
    fn class(&mut self, td: &str, div: &str) -> String {
        if td.is_empty() && div.is_empty() {
            return String::new();
        }
        let key = format!("{td}|{div}");
        if let Some(i) = self.by_rule.get(&key) {
            return format!("k{i}");
        }
        let i = self.by_rule.len();
        self.by_rule.insert(key, i);
        if !td.is_empty() {
            let _ = writeln!(self.css, ".xljs-sheet td.k{i}{{{td}}}");
        }
        if !div.is_empty() {
            let _ = writeln!(self.css, ".xljs-sheet td.k{i}>div{{{div}}}");
        }
        format!("k{i}")
    }
}

const BASE_CSS: &str = r#"html,body{margin:0;padding:0;background:#fff;color:#000}
body{-webkit-print-color-adjust:exact;print-color-adjust:exact}
.xljs-page{position:relative;overflow:hidden;break-after:page;display:flex;flex-direction:column;box-sizing:border-box;width:var(--w);height:var(--h)}
.xljs-page:last-child{break-after:auto}
.xljs-sheet{position:relative;flex:none}
.xljs-sheet table{border-collapse:collapse;table-layout:fixed}
.xljs-sheet td{padding:0;position:relative;overflow:visible}
.xljs-sheet td>div{position:absolute;top:0;display:flex;flex-direction:column;justify-content:flex-end;overflow:hidden;box-sizing:border-box;padding:0 1.5pt;white-space:pre;line-height:normal}
.xljs-sheet.grid td{border:0.5pt solid #c8c8c8}
.xljs-objs{position:absolute;overflow:hidden;pointer-events:none}
.xljs-objs>*{position:absolute}
.xljs-objs img{display:block}
.xljs-print-chart{overflow:hidden}
.xljs-shape{position:absolute;box-sizing:border-box}
.xljs-shape p{margin:0}
.xljs-print-chart .xljs-chart-error{display:flex;align-items:center;justify-content:center;height:100%;border:0.75pt dashed #999;color:#666;font-size:9pt;box-sizing:border-box;padding:4pt;text-align:center}
@media screen{
body{background:#e6e6e6;padding:12px 0}
.xljs-page{width:var(--pw);height:var(--ph);padding:var(--m);margin:0 auto 12px;background:#fff;box-shadow:0 1px 4px rgba(0,0,0,.25)}
}
"#;

/// Lays out the sheets of a workbook (see PrintOut).
pub fn workbook_html(b: &Book, o: &PrintOptions) -> Result<PrintOut> {
    let names: Vec<String> = if o.sheets.is_empty() {
        b.wb.sheet_collection()
            .iter()
            .map(|s| s.name().to_string())
            .filter(|name| !b.print.get(name).is_some_and(|p| p.hidden))
            .collect()
    } else {
        o.sheets.clone()
    };
    let now = {
        let t = chrono::Local::now();
        (t.format("%Y/%m/%d").to_string(), t.format("%H:%M").to_string())
    };
    let path = std::path::Path::new(&b.path);
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let mut classes = Classes::default();
    let mut page_css = String::new();
    let mut body = String::new();
    let mut charts = Vec::new();
    let mut pages = 0;
    let ids = std::cell::Cell::new(0);
    for (si, name) in names.iter().enumerate() {
        let sh = Sheet::new(b, name)?;
        let setup = setup_of(&sh, b.print.get(name), o)?;
        // The drawing's objects as (anchor, opening tag, content), later ones on top.
        let mut objects: Vec<(Anchor, String, String)> = Vec::new();
        match super::xlsx_shape::sheet_drawing(b, name, &ids, &|an| sh.anchor_rect(an).map(|r| (r.2, r.3))) {
            Some(drawn) => {
                for d in drawn {
                    match d.drawn {
                        super::xlsx_shape::Drawn::Chart(i) => {
                            if o.charts == Some(false) {
                                continue;
                            }
                            let k = charts.len();
                            charts.push(ChartRef { sheet: name.clone(), index: i });
                            objects.push((d.anchor, format!("<div class=\"xljs-print-chart\" data-chart=\"{k}\""), String::new()));
                        }
                        super::xlsx_shape::Drawn::Html(h) => objects.push((d.anchor, "<div class=\"xljs-print-shape\"".into(), h)),
                    }
                }
            }
            // A new workbook: the pictures umya-spreadsheet keeps, if any.
            None => {
                for img in sh.ws.image_collection() {
                    if !img.has_image() {
                        continue;
                    }
                    let m = |mk: &umya_spreadsheet::structs::drawing::spreadsheet::MarkerType| (mk.col(), mk.col_off() as i64, mk.row(), mk.row_off() as i64);
                    let mut an = Anchor { from: Some(m(img.from_marker_type())), ..Default::default() };
                    if let Some(t) = img.to_marker_type() {
                        an.to = Some(m(t));
                    } else if let Some(one) = img.one_cell_anchor() {
                        an.ext = Some((one.extent().cx(), one.extent().cy()));
                    }
                    let lower = img.image_name().to_lowercase();
                    let mime = if lower.ends_with(".png") {
                        "image/png"
                    } else if lower.ends_with(".gif") {
                        "image/gif"
                    } else if lower.ends_with(".bmp") {
                        "image/bmp"
                    } else if lower.ends_with(".svg") {
                        "image/svg+xml"
                    } else {
                        "image/jpeg"
                    };
                    let data = base64::engine::general_purpose::STANDARD.encode(img.image_data());
                    objects.push((an, format!("<img alt=\"\" src=\"data:{mime};base64,{data}\""), String::new()));
                }
            }
        }
        let all_anchors: Vec<Anchor> = objects.iter().map(|(a, _, _)| a.clone()).collect();
        let areas = sh.areas(&all_anchors);
        if areas.is_empty() {
            continue;
        }
        let page_name = format!("s{si}");
        page_css.push_str(&page_rule(&page_name, &sh, &setup, &file, &dir, &now));
        for area in areas {
            pages += sheet_pages(&sh, &setup, area, &objects, &page_name, &mut classes, &mut body);
        }
    }
    if pages == 0 {
        return Err(err!("pdf.empty"));
    }
    let title = if o.title.is_empty() { file.clone() } else { o.title.clone() };
    let lang = if crate::i18n::lang() == "ja" { "ja" } else { "en" };
    let mut html = String::with_capacity(body.len() + 4096);
    let _ = write!(
        html,
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\n{BASE_CSS}{page_css}{}</style>\n</head>\n<body class=\"xljs-print\">\n{body}</body>\n</html>\n",
        esc(&title),
        classes.css
    );
    Ok(PrintOut { html, charts, pages })
}

fn setup_of(sh: &Sheet, raw: Option<&RawPrint>, o: &PrintOptions) -> Result<Setup> {
    let ps = sh.ws.page_setup();
    let code = match &o.paper {
        Some(p) => paper_code(p).ok_or_else(|| err!("pdf.badPaper", p))?,
        None => ps.paper_size(),
    };
    let (mut w, mut h) = paper_size(code);
    let landscape = o.landscape.unwrap_or(ps.orientation().value_string() == "landscape");
    if landscape != (w > h) {
        std::mem::swap(&mut w, &mut h);
    }
    let pm = sh.ws.page_margins();
    let inch = |has: bool, v: f64, def: f64| if has { v * 72.0 } else { def * 72.0 };
    let mut margins = [
        inch(pm.has_top(), pm.top(), 0.75),
        inch(pm.has_right(), pm.right(), 0.7),
        inch(pm.has_bottom(), pm.bottom(), 0.75),
        inch(pm.has_left(), pm.left(), 0.7),
    ];
    if let Some(m) = &o.margin {
        for (i, v) in [m.top, m.right, m.bottom, m.left].into_iter().enumerate() {
            if let Some(v) = v {
                margins[i] = v.max(0.0) * PT_MM;
            }
        }
    }
    let raw = raw.cloned().unwrap_or_default();
    let (scale, fit) = if o.fit_to_width.is_some() || o.fit_to_height.is_some() {
        (None, Some((o.fit_to_width.unwrap_or(0), o.fit_to_height.unwrap_or(0))))
    } else if let Some(s) = o.scale {
        (Some((s / 100.0).clamp(0.1, 4.0)), None)
    } else if ps.fit_to_page() {
        (None, Some((raw.fit_width.unwrap_or(1), raw.fit_height.unwrap_or(1))))
    } else {
        let s = ps.scale();
        (Some(if s == 0 { 1.0 } else { (s as f64 / 100.0).clamp(0.1, 4.0) }), None)
    };
    let fit = fit.filter(|f| f.0 > 0 || f.1 > 0);
    Ok(Setup {
        paper: (w * PT_MM, h * PT_MM),
        margins,
        header: inch(pm.has_header(), pm.header(), 0.3),
        footer: inch(pm.has_footer(), pm.footer(), 0.3),
        scale: if fit.is_some() { None } else { scale.or(Some(1.0)) },
        fit,
        over_then_down: raw.over_then_down,
        grid: o.grid_lines.unwrap_or(raw.grid_lines),
        center_h: o.center_horizontally.unwrap_or(sh.ws.print_options().horizontal_centered()),
        center_v: sh.ws.print_options().vertical_centered(),
    })
}

/// The @page rule of a sheet: size, margins, header and footer.
fn page_rule(name: &str, sh: &Sheet, s: &Setup, file: &str, dir: &str, now: &(String, String)) -> String {
    let [t, r, bt, l] = s.margins;
    let mut css = format!("@page {name}{{size:{}pt {}pt;margin:{}pt {}pt {}pt {}pt;", n(s.paper.0), n(s.paper.1), n(t), n(r), n(bt), n(l));
    let hf = sh.ws.header_footer();
    let font = format!("font-family:{};font-size:{}pt;", font_family(&sh.font.0), n(sh.font.1));
    for (text, top) in [(hf.odd_header().value(), true), (hf.odd_footer().value(), false)] {
        if text.is_empty() {
            continue;
        }
        for (i, sec) in hf_sections(text).iter().enumerate() {
            if sec.is_empty() {
                continue;
            }
            let (content, extra) = hf_content(sec, &sh.name, file, dir, now);
            if content.is_empty() {
                continue;
            }
            let pos = ["left", "center", "right"][i];
            let edge = if top { "top" } else { "bottom" };
            let pad = if top { format!("padding-top:{}pt;vertical-align:top;", n(s.header)) } else { format!("padding-bottom:{}pt;vertical-align:bottom;", n(s.footer)) };
            let _ = write!(css, "@{edge}-{pos}{{content:{content};text-align:{pos};white-space:pre;{font}{pad}{extra}}}");
        }
    }
    css.push_str("}\n");
    css
}

/// Writes the pages of one area of a sheet; returns how many.
fn sheet_pages(sh: &Sheet, s: &Setup, area: Area, objects: &[(Anchor, String, String)], page: &str, classes: &mut Classes, out: &mut String) -> usize {
    let (c1, r1, c2, r2) = area;
    let (title_rows, title_cols) = sh.titles();
    let sum = |a: u32, b: u32, f: &dyn Fn(u32) -> f64| (a..=b).map(f).sum::<f64>();
    let tr_h = title_rows.map(|(a, b)| sum(a, b, &|r| sh.h(r))).unwrap_or(0.0);
    let tc_w = title_cols.map(|(a, b)| sum(a, b, &|c| sh.w(c))).unwrap_or(0.0);
    let avail_w = s.paper.0 - s.margins[1] - s.margins[3];
    let avail_h = s.paper.1 - s.margins[0] - s.margins[2];
    let (scale, col_breaks, row_breaks) = match s.fit {
        Some((fw, fh)) => {
            let total_w = sum(c1, c2, &|c| sh.w(c)).max(1.0);
            let total_h = sum(r1, r2, &|r| sh.h(r)).max(1.0);
            let mut k: f64 = 1.0;
            if fw > 0 {
                k = k.min(avail_w * fw as f64 / total_w);
            }
            if fh > 0 {
                k = k.min(avail_h * fh as f64 / total_h);
            }
            ((k * 100.0).floor().max(10.0) / 100.0, HashSet::new(), HashSet::new())
        }
        None => (s.scale.unwrap_or(1.0), sh.breaks(false), sh.breaks(true)),
    };
    let col_segs = split(c1, c2, &|c| sh.w(c), avail_w / scale, tc_w, title_cols.map(|t| t.1).unwrap_or(0), &col_breaks);
    let row_segs = split(r1, r2, &|r| sh.h(r), avail_h / scale, tr_h, title_rows.map(|t| t.1).unwrap_or(0), &row_breaks);
    let mut order = Vec::new();
    if s.over_then_down {
        for rs in &row_segs {
            for cs in &col_segs {
                order.push((*cs, *rs));
            }
        }
    } else {
        for cs in &col_segs {
            for rs in &row_segs {
                order.push((*cs, *rs));
            }
        }
    }
    let [mt, mr, mb, ml] = s.margins;
    let style = format!(
        "page:{page};--w:{}pt;--h:{}pt;--pw:{}pt;--ph:{}pt;--m:{}pt {}pt {}pt {}pt;align-items:{};justify-content:{}",
        n(avail_w),
        n(avail_h - 1.0),
        n(s.paper.0),
        n(s.paper.1),
        n(mt),
        n(mr),
        n(mb),
        n(ml),
        if s.center_h { "center" } else { "flex-start" },
        if s.center_v { "center" } else { "flex-start" }
    );
    for &(cs, rs) in &order {
        let mut cols: Vec<u32> = Vec::new();
        let mut body_x = 0.0;
        if let Some((a, b)) = title_cols
            && cs.0 > b
        {
            cols.extend((a..=b).filter(|c| sh.w(*c) > 0.0));
            body_x = tc_w;
        }
        cols.extend((cs.0..=cs.1).filter(|c| sh.w(*c) > 0.0));
        let mut rows: Vec<u32> = Vec::new();
        let mut body_y = 0.0;
        if let Some((a, b)) = title_rows
            && rs.0 > b
        {
            rows.extend((a..=b).filter(|r| sh.h(*r) > 0.0));
            body_y = tr_h;
        }
        rows.extend((rs.0..=rs.1).filter(|r| sh.h(*r) > 0.0));
        let _ = write!(out, "<section class=\"xljs-page\" style=\"{style}\">\n<div class=\"xljs-sheet{}\" style=\"zoom:{}\">", if s.grid { " grid" } else { "" }, n(scale));
        page_table(sh, &cols, &rows, classes, out);
        // Pictures, shapes and charts, cut to the body of the page.
        let (bx0, by0) = (sh.x0(cs.0 - 1), sh.y0(rs.0 - 1));
        let (bw, bh) = (sh.x0(cs.1) - bx0, sh.y0(rs.1) - by0);
        let mut objs = String::new();
        for (an, tag, inner) in objects {
            let Some((x, y, w, h)) = sh.anchor_rect(an) else { continue };
            let (x, y) = (x - bx0, y - by0);
            if x >= bw || y >= bh || x + w <= 0.0 || y + h <= 0.0 {
                continue;
            }
            let _ = write!(objs, "{tag} style=\"left:{}pt;top:{}pt;width:{}pt;height:{}pt\"", n(x), n(y), n(w), n(h));
            if tag.starts_with("<img") {
                objs.push('>');
            } else {
                let _ = write!(objs, ">{inner}</div>");
            }
        }
        if !objs.is_empty() {
            let _ = write!(out, "<div class=\"xljs-objs\" style=\"left:{}pt;top:{}pt;width:{}pt;height:{}pt\">{objs}</div>", n(body_x), n(body_y), n(bw), n(bh));
        }
        out.push_str("</div>\n</section>\n");
    }
    order.len()
}

/// The CSS of a cell: (td: fill and borders, div: font and alignment).
struct CellCss {
    td: String,
    div: String,
    wrap: bool,
    rotated: bool,
    halign: String,
}

fn side_css(sh: &Sheet, st: &Style, side: &str) -> Option<String> {
    let b = st.borders()?;
    let bd = match side {
        "left" => b.left(),
        "right" => b.right(),
        "top" => b.top(),
        _ => b.bottom(),
    };
    let style = bd.border_style();
    let color = bd.color().as_ref().and_then(|c| css_color(c, sh.theme)).unwrap_or_else(|| "#000".into());
    border_css(style, &color)
}

fn cell_css(sh: &Sheet, st: &Style, val: &Val, sides: [Option<String>; 4]) -> CellCss {
    let mut td = String::new();
    let mut div = String::new();
    let fill = sh.fill(st);
    let mut bg: Vec<String> = Vec::new();
    if let Some(b) = st.borders() {
        let ds = b.diagonal().border_style();
        if (b.diagonal_up() || b.diagonal_down()) && !ds.is_empty() && ds != "none" {
            let color = b.diagonal().color().as_ref().and_then(|c| css_color(c, sh.theme)).unwrap_or_else(|| "#000".into());
            let line = |dir: &str| format!("linear-gradient(to {dir},transparent calc(50% - .5pt),{color} calc(50% - .5pt),{color} calc(50% + .5pt),transparent calc(50% + .5pt))");
            if b.diagonal_up() {
                bg.push(line("top left"));
            }
            if b.diagonal_down() {
                bg.push(line("top right"));
            }
        }
    }
    if let Some(f) = &fill {
        let _ = write!(td, "background-color:{f};");
    }
    if !bg.is_empty() {
        let _ = write!(td, "background-image:{};", bg.join(","));
    }
    for (i, side) in ["top", "right", "bottom", "left"].into_iter().enumerate() {
        if let Some(v) = &sides[i] {
            let _ = write!(td, "border-{side}:{v};");
        }
    }
    if let Some(f) = st.font() {
        let name = if f.name().is_empty() { sh.font.0.as_str() } else { f.name() };
        let size = if f.size() == 0.0 { sh.font.1 } else { f.size() };
        if name != sh.font.0 {
            let _ = write!(div, "font-family:{};", font_family(name));
        }
        if size != sh.font.1 {
            let _ = write!(div, "font-size:{}pt;", n(size));
        }
        if f.bold() {
            div.push_str("font-weight:bold;");
        }
        if f.italic() {
            div.push_str("font-style:italic;");
        }
        let u = f.font_underline().val().value_string();
        let mut deco = Vec::new();
        if u != "none" && !u.is_empty() {
            deco.push("underline");
        }
        if f.strikethrough() {
            deco.push("line-through");
        }
        if !deco.is_empty() {
            let _ = write!(div, "text-decoration:{};", deco.join(" "));
            if u.starts_with("double") {
                div.push_str("text-decoration-style:double;");
            }
        }
        if let Some(c) = css_color(f.color(), sh.theme)
            && c != "#000000"
        {
            let _ = write!(div, "color:{c};");
        }
    }
    let code = format_code(st.number_format());
    let fc = match val {
        Val::Num(f, _) => format_color(&code, Some(*f)),
        Val::Text(_) => format_color(&code, None),
        _ => None,
    };
    if let Some(c) = fc {
        let _ = write!(div, "color:{c};");
    }
    let (mut h, mut v, mut wrap, mut rot, mut indent) = (String::from("general"), String::from("bottom"), false, 0, 0);
    if let Some(a) = st.alignment() {
        h = a.horizontal().value_string().to_string();
        v = a.vertical().value_string().to_string();
        wrap = a.wrap_text();
        rot = a.text_rotation();
        indent = a.indent();
    }
    let halign = match h.as_str() {
        "general" | "" => match val {
            Val::Num(..) => "right",
            Val::Bool(_) | Val::Error(_) => "center",
            _ => "left",
        },
        "center" | "centerContinuous" => "center",
        "right" => "right",
        "justify" | "distributed" => "justify",
        "fill" => "left",
        _ => "left",
    }
    .to_string();
    if halign != "left" {
        let _ = write!(div, "text-align:{halign};");
    }
    match v.as_str() {
        "top" => div.push_str("justify-content:flex-start;"),
        "center" | "justify" | "distributed" => div.push_str("justify-content:center;"),
        _ => {}
    }
    if wrap || h == "justify" || h == "distributed" || v == "justify" || v == "distributed" {
        wrap = true;
        div.push_str("white-space:pre-wrap;overflow-wrap:anywhere;");
    }
    if indent > 0 {
        let pad = indent as f64 * sh.mdw * 1.4 * 0.75 + 1.5;
        let side = if halign == "right" { "right" } else { "left" };
        let _ = write!(div, "padding-{side}:{}pt;", n(pad));
    }
    let rotated = rot != 0;
    if rot == 255 {
        div.push_str("writing-mode:vertical-rl;text-orientation:upright;white-space:pre;align-items:center;justify-content:center;");
    } else if rotated {
        div.push_str("align-items:center;justify-content:center;text-align:center;");
    }
    CellCss { td, div, wrap, rotated, halign }
}

/// The text of a cell as HTML (rich text runs with their fonts).
fn cell_html(sh: &Sheet, col: u32, row: u32, val: &Val, wrap: bool) -> String {
    let line = |s: &str| if wrap { esc(s) } else { esc(&s.replace("\r\n", "").replace('\n', "")) };
    if let Val::Text(_) = val
        && let Some(c) = sh.cells.get(&(col, row))
        && let CellRawValue::RichText(rt) = c.raw_value()
    {
        let mut out = String::new();
        for el in rt.rich_text_elements() {
            let mut css = String::new();
            if let Some(f) = el.run_properties() {
                if !f.name().is_empty() && f.name() != sh.font.0 {
                    let _ = write!(css, "font-family:{};", font_family(f.name()));
                }
                if f.size() > 0.0 {
                    let _ = write!(css, "font-size:{}pt;", n(f.size()));
                }
                if f.bold() {
                    css.push_str("font-weight:bold;");
                }
                if f.italic() {
                    css.push_str("font-style:italic;");
                }
                let u = f.font_underline().val().value_string();
                if u != "none" && !u.is_empty() {
                    css.push_str("text-decoration:underline;");
                }
                if f.strikethrough() {
                    css.push_str("text-decoration:line-through;");
                }
                match f.vertical_text_alignment().val().value_string() {
                    "superscript" => css.push_str("vertical-align:super;font-size:smaller;"),
                    "subscript" => css.push_str("vertical-align:sub;font-size:smaller;"),
                    _ => {}
                }
                if let Some(c) = css_color(f.color(), sh.theme) {
                    let _ = write!(css, "color:{c};");
                }
            }
            if css.is_empty() {
                out.push_str(&line(el.text()));
            } else {
                let _ = write!(out, "<span style=\"{css}\">{}</span>", line(el.text()));
            }
        }
        return out;
    }
    match val {
        Val::Empty => String::new(),
        Val::Num(_, s) => esc(s),
        Val::Text(s) => line(s),
        Val::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Val::Error(e) => esc(e),
    }
}

/// Writes the table of the cells cols × rows (sheet numbers, in order).
fn page_table(sh: &Sheet, cols: &[u32], rows: &[u32], classes: &mut Classes, out: &mut String) {
    let total: f64 = cols.iter().map(|c| sh.w(*c)).sum();
    let _ = write!(out, "<table style=\"width:{}pt;font-family:{};font-size:{}pt\"><colgroup>", n(total), esc(&font_family(&sh.font.0)), n(sh.font.1));
    for c in cols {
        let _ = write!(out, "<col style=\"width:{}pt\">", n(sh.w(*c)));
    }
    out.push_str("</colgroup>\n");
    let mut covered: HashSet<(usize, usize)> = HashSet::new();
    let empty = |c: u32, r: u32| sh.val(c, r) == Val::Empty && !sh.merge_at.contains_key(&(c, r));
    for (ri, &r) in rows.iter().enumerate() {
        let _ = write!(out, "<tr style=\"height:{}pt\">", n(sh.h(r)));
        for (ci, &c) in cols.iter().enumerate() {
            if covered.contains(&(ci, ri)) {
                continue;
            }
            let merge = sh.merge_at.get(&(c, r)).map(|i| sh.merges[*i]);
            let (vc, vr, cspan, rspan) = match merge {
                Some(m) => {
                    let mut cs = 1;
                    while ci + cs < cols.len() && cols[ci + cs] <= m.2 && cols[ci + cs] >= m.0 {
                        cs += 1;
                    }
                    let mut rs = 1;
                    while ri + rs < rows.len() && rows[ri + rs] <= m.3 && rows[ri + rs] >= m.1 {
                        rs += 1;
                    }
                    for y in 0..rs {
                        for x in 0..cs {
                            covered.insert((ci + x, ri + y));
                        }
                    }
                    (m.0, m.1, cs, rs)
                }
                None => (c, r, 1, 1),
            };
            let st_of = |c: u32, r: u32| sh.cells.get(&(c, r)).map(|x| x.style());
            let default_style = Style::default();
            let st = st_of(vc, vr).or_else(|| sh.ws.row_dimension(r).map(|x| x.style())).unwrap_or(&default_style);
            let side = |cc: u32, rr: u32, s: &str| st_of(cc, rr).and_then(|x| side_css(sh, x, s));
            let sides = match merge {
                Some(m) => [side(m.0, m.1, "top"), side(m.2, m.1, "right"), side(m.0, m.3, "bottom"), side(m.0, m.1, "left")],
                None => [side(c, r, "top"), side(c, r, "right"), side(c, r, "bottom"), side(c, r, "left")],
            };
            let val = sh.val(vc, vr);
            let css = cell_css(sh, st, &val, sides);
            let class = classes.class(&css.td, &css.div);
            out.push_str("<td");
            if !class.is_empty() {
                let _ = write!(out, " class=\"{class}\"");
            }
            if cspan > 1 {
                let _ = write!(out, " colspan=\"{cspan}\"");
            }
            if rspan > 1 {
                let _ = write!(out, " rowspan=\"{rspan}\"");
            }
            out.push('>');
            if val != Val::Empty {
                let w: f64 = cols[ci..ci + cspan].iter().map(|c| sh.w(*c)).sum();
                let h: f64 = rows[ri..ri + rspan].iter().map(|r| sh.h(*r)).sum();
                // Text overflows into the empty cells beside it, as in Excel.
                let (mut left, mut right) = (0.0, 0.0);
                if matches!(val, Val::Text(_)) && merge.is_none() && !css.wrap && !css.rotated {
                    let grow = |dir: i64| -> f64 {
                        let mut s = 0.0;
                        let mut i = ci as i64 + dir;
                        while i >= 0 && (i as usize) < cols.len() && empty(cols[i as usize], r) {
                            s += sh.w(cols[i as usize]);
                            i += dir;
                        }
                        s
                    };
                    match css.halign.as_str() {
                        "left" => right = grow(1),
                        "right" => left = grow(-1),
                        "center" => {
                            let m = grow(1).min(grow(-1));
                            left = m;
                            right = m;
                        }
                        _ => {}
                    }
                }
                let _ = write!(out, "<div style=\"left:{}pt;width:{}pt;height:{}pt\">", n(-left), n(w + left + right), n(h));
                let text = cell_html(sh, vc, vr, &val, css.wrap);
                if css.rotated && st.alignment().map(|a| a.text_rotation()).unwrap_or(0) != 255 {
                    let rot = st.alignment().map(|a| a.text_rotation()).unwrap_or(0);
                    let deg = if rot <= 90 { -(rot as f64) } else { rot as f64 - 90.0 };
                    let _ = write!(out, "<span style=\"display:inline-block;transform:rotate({}deg)\">{text}</span>", n(deg));
                } else {
                    out.push_str(&text);
                }
                out.push_str("</div>");
            }
            out.push_str("</td>");
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_and_refs() {
        assert_eq!(parse_area("'集計'!$A$1:$H$40", 16384, 1048576), Some((1, 1, 8, 40)));
        assert_eq!(parse_area("Sheet1!$1:$2", 16384, 1048576), Some((1, 1, 16384, 2)));
        assert_eq!(parse_area("Sheet1!$A:$B", 16384, 1048576), Some((1, 1, 2, 1048576)));
        assert_eq!(parse_area("B3", 16384, 1048576), Some((2, 3, 2, 3)));
        assert_eq!(split_refs("'a,b'!$A$1:$B$2,'a,b'!$D$1:$E$2"), vec!["'a,b'!$A$1:$B$2", "'a,b'!$D$1:$E$2"]);
    }

    #[test]
    fn splitting() {
        let size = |_: u32| 10.0;
        let none = HashSet::new();
        assert_eq!(split(1, 10, &size, 35.0, 0.0, 0, &none), vec![(1, 3), (4, 6), (7, 9), (10, 10)]);
        // Titles (rows 1-2, 20pt) before every page after the first.
        assert_eq!(split(1, 10, &size, 40.0, 20.0, 2, &none), vec![(1, 4), (5, 6), (7, 8), (9, 10)]);
        let breaks: HashSet<u32> = [3].into_iter().collect();
        assert_eq!(split(1, 5, &size, 100.0, 0.0, 0, &breaks), vec![(1, 2), (3, 5)]);
        // Hidden rows take no room.
        let hidden = |i: u32| if i == 4 { 0.0 } else { 10.0 };
        assert_eq!(split(1, 6, &hidden, 30.0, 0.0, 0, &none), vec![(1, 4), (5, 6)]);
    }

    #[test]
    fn headers() {
        let now = ("2026/10/31".to_string(), "12:00".to_string());
        let s = hf_sections("&L&\"游ゴシック,太字\"見積書&C&P / &N ページ&R&D");
        assert_eq!(s[0], "&\"游ゴシック,太字\"見積書");
        let (c, css) = hf_content(&s[0], "S", "a.xlsx", "C:\\x", &now);
        assert_eq!(c, "\"見積書\"");
        assert!(css.contains("font-weight:bold"), "{css}");
        let (c, _) = hf_content(&s[1], "S", "a.xlsx", "", &now);
        assert_eq!(c, "counter(page) \" / \" counter(pages) \" ページ\"");
        assert_eq!(hf_content(&s[2], "S", "", "", &now).0, "\"2026/10/31\"");
        assert_eq!(hf_content("&A &F &&", "集計", "a.xlsx", "", &now).0, "\"集計 a.xlsx &\"");
    }

    #[test]
    fn colors_of_formats() {
        assert_eq!(format_color("#,##0;[Red]-#,##0", Some(-1.0)), Some("#FF0000"));
        assert_eq!(format_color("#,##0;[赤]-#,##0", Some(1.0)), None);
        assert_eq!(format_color("[Blue]0", Some(1.0)), Some("#0000FF"));
    }

    #[test]
    fn raw_page_setup() {
        let xml = br#"<worksheet><sheetPr><pageSetUpPr fitToPage="1"/></sheetPr><sheetData><row r="1"/></sheetData><printOptions gridLines="1"/><pageSetup paperSize="9" orientation="landscape" fitToHeight="0" pageOrder="overThenDown"/></worksheet>"#;
        let r = raw_print(xml);
        assert_eq!(r.fit_width, None);
        assert_eq!(r.fit_height, Some(0));
        assert!(r.over_then_down && r.grid_lines);
    }
}

pub(super) fn install(n: &super::N) {
    // xlsxPrintHtml(book, optionsJSON) -> JSON { html, charts: [{ sheet, index }], pages }
    n.set("xlsxPrintHtml", |h, a| {
        let o: PrintOptions = serde_json::from_str(&a.str(1)).map_err(|e| a.throw(&crate::Error::from(e)))?;
        let out = super::xlsx::with_book(h, a, |b| workbook_html(b, &o).map_err(|e| a.throw(&e)))?;
        a.ret(serde_json::json!({ "html": out.html, "charts": out.charts, "pages": out.pages }).to_string())
    });
}
