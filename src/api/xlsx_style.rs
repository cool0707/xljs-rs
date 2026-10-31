//! Cell styles: parsing the style objects of the scripts, merging them
//! into the cells' styles, and reading styles back.

use std::collections::BTreeMap;
use std::str::FromStr;

use rquickjs::Value;
use umya_spreadsheet::structs::EnumTrait;
use umya_spreadsheet::{
    Border, BorderStyleValues, HorizontalAlignmentValues, NumberingFormat, PatternValues, Style, UnderlineValues,
    VerticalAlignmentValues,
};
use unicode_width::UnicodeWidthStr;

use super::js::{self, get, has, nullish, object, to_bool, to_f64, to_str, v};
use super::xlsx::{self, Book, cell, column_name, parse_ref, with_book};
use super::{N, R};
use crate::{Error, Result, err};

/// A side of a border: style 0 removes it.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderSide {
    pub style: usize,
    pub color: String,
}

/// The Excel border styles; the index is the style id (as excelize has it).
const BORDER_NAMES: [&str; 14] = [
    "none", "thin", "medium", "dashed", "dotted", "thick", "double", "hair", "mediumDashed", "dashDot", "mediumDashDot", "dashDotDot",
    "mediumDashDotDot", "slantDashDot",
];

const SIDES: [&str; 6] = ["left", "right", "top", "bottom", "diagonalUp", "diagonalDown"];

#[derive(Default, Clone)]
struct FontSpec {
    name: Option<String>,
    size: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    strike: Option<bool>,
    underline: Option<String>,
    color: Option<String>,
    vert_align: Option<String>,
}

#[derive(Default, Clone)]
struct AlignSpec {
    horizontal: Option<String>,
    vertical: Option<String>,
    wrap: Option<bool>,
    indent: Option<i64>,
    rotation: Option<i64>,
}

/// A partial cell style from JavaScript. None fields are left unchanged
/// when the spec is merged into a cell's existing style.
#[derive(Default, Clone)]
pub struct StyleSpec {
    font: Option<FontSpec>,
    fill: Option<String>, // "" removes the fill
    align: Option<AlignSpec>,
    num_fmt: Option<String>,
    num_fmt_id: Option<u32>,
    borders: Option<BTreeMap<String, BorderSide>>, // per side for every cell
    outline: Option<BorderSide>,                  // outer edges of the range
    inside: Option<BorderSide>,                   // edges between cells of the range
    inside_h: Option<BorderSide>,
    inside_v: Option<BorderSide>,
    protected: Option<bool>,
}

fn named_color(c: &str) -> Option<&'static str> {
    Some(match c {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "orange" => "FFA500",
        "gray" | "grey" => "808080",
        "silver" => "C0C0C0",
        "navy" => "000080",
        "purple" => "800080",
        "teal" => "008080",
        "lime" => "00FF00",
        "aqua" | "cyan" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "maroon" => "800000",
        "olive" => "808000",
        "lightgray" | "lightgrey" => "D3D3D3",
        "darkgray" | "darkgrey" => "A9A9A9",
        _ => return None,
    })
}

/// Converts "#RGB", "#RRGGBB", "RRGGBB", "#AARRGGBB" or a CSS-like color
/// name to "RRGGBB".
pub fn norm_color(s: &str) -> Result<String> {
    let c = s.trim().to_lowercase();
    if let Some(v) = named_color(&c) {
        return Ok(v.to_string());
    }
    let mut c = c.strip_prefix('#').unwrap_or(&c).to_string();
    match c.len() {
        3 => c = c.chars().flat_map(|ch| [ch, ch]).collect(),
        8 => c = c[2..].to_string(),
        _ => {}
    }
    if c.len() != 6 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(err!("xlsx.invalidColor", s));
    }
    Ok(c.to_uppercase())
}

// ---- parsing from JavaScript ----

fn opt_s(o: &Value, k: &str) -> Option<String> {
    if !has(o, k) {
        return None;
    }
    let x = get(o, k);
    Some(if x.is_null() { String::new() } else { to_str(&x) })
}

fn opt_b(o: &Value, k: &str) -> Option<bool> {
    if has(o, k) { Some(to_bool(&get(o, k))) } else { None }
}

fn opt_n(o: &Value, k: &str) -> Option<f64> {
    let x = get(o, k);
    if nullish(&x) { None } else { Some(to_f64(&x)) }
}

fn opt_i(o: &Value, k: &str) -> Option<i64> {
    opt_n(o, k).map(|f| f.round() as i64)
}

fn is_obj(x: &Value) -> bool {
    x.is_object() && !x.is_function()
}

fn parse_border_style(s: &str) -> Result<usize> {
    let l = s.trim().to_lowercase();
    BORDER_NAMES.iter().position(|n| n.to_lowercase() == l).ok_or_else(|| err!("xlsx.invalidBorder", s))
}

/// Parses "thin", true, null/"none", or {style, color}.
fn border_side_of(x: &Value) -> Result<Option<BorderSide>> {
    if x.is_undefined() {
        return Ok(None);
    }
    if x.is_null() {
        return Ok(Some(BorderSide { style: 0, color: String::new() }));
    }
    if let Some(b) = x.as_bool() {
        return Ok(Some(if b { BorderSide { style: 1, color: "000000".into() } } else { BorderSide { style: 0, color: String::new() } }));
    }
    if x.is_string() {
        return Ok(Some(BorderSide { style: parse_border_style(&to_str(x))?, color: "000000".into() }));
    }
    if !is_obj(x) {
        return Err(err!("xlsx.invalidBorder", to_str(x)));
    }
    let mut side = BorderSide { style: 1, color: "000000".into() };
    if let Some(s) = opt_s(x, "style") {
        side.style = parse_border_style(&s)?;
    }
    if let Some(c) = opt_s(x, "color")
        && !c.is_empty()
    {
        side.color = norm_color(&c)?;
    }
    Ok(Some(side))
}

pub fn parse_style(x: &Value) -> Result<StyleSpec> {
    let mut sp = StyleSpec::default();
    if nullish(x) || !is_obj(x) {
        return Ok(sp);
    }
    let f = get(x, "font");
    if is_obj(&f) && !nullish(&f) {
        let mut fs = FontSpec {
            name: opt_s(&f, "name"),
            size: opt_n(&f, "size"),
            bold: opt_b(&f, "bold"),
            italic: opt_b(&f, "italic"),
            strike: opt_b(&f, "strike"),
            vert_align: opt_s(&f, "vertAlign"),
            ..Default::default()
        };
        if has(&f, "underline") {
            let u = get(&f, "underline");
            let s = if let Some(b) = u.as_bool() {
                if b { "single".to_string() } else { String::new() }
            } else if u.is_string() {
                let s = to_str(&u);
                if s == "none" { String::new() } else { s }
            } else {
                String::new()
            };
            fs.underline = Some(s);
        }
        if let Some(c) = opt_s(&f, "color") {
            fs.color = Some(if c.is_empty() { String::new() } else { norm_color(&c)? });
        }
        sp.font = Some(fs);
    }
    if has(x, "fill") {
        let fv = get(x, "fill");
        let mut s = String::new();
        if is_obj(&fv) && !fv.is_null() {
            if let Some(c) = opt_s(&fv, "color") {
                s = c;
            }
        } else if !fv.is_null() {
            s = to_str(&fv);
        }
        let s = if !s.is_empty() && !s.eq_ignore_ascii_case("none") { norm_color(&s)? } else { String::new() };
        sp.fill = Some(s);
    }
    let al = get(x, "alignment");
    if is_obj(&al) && !nullish(&al) {
        let mut a = AlignSpec {
            horizontal: opt_s(&al, "horizontal"),
            vertical: opt_s(&al, "vertical"),
            wrap: opt_b(&al, "wrap"),
            indent: opt_i(&al, "indent"),
            rotation: opt_i(&al, "rotation"),
        };
        if a.vertical.as_deref() == Some("middle") {
            a.vertical = Some("center".into());
        }
        sp.align = Some(a);
    }
    if has(x, "numberFormat") {
        let nv = get(x, "numberFormat");
        match nv.as_int() {
            Some(i) if i >= 0 => sp.num_fmt_id = Some(i as u32),
            _ => {
                if let Some(f) = nv.as_float().filter(|f| *f == f.trunc() && *f >= 0.0) {
                    sp.num_fmt_id = Some(f as u32);
                } else {
                    sp.num_fmt = Some(if nv.is_null() { String::new() } else { to_str(&nv) });
                }
            }
        }
    }
    if has(x, "border") {
        let bv = get(x, "border");
        let shorthand = !is_obj(&bv) || bv.is_null() || has(&bv, "style") || has(&bv, "color");
        if shorthand {
            // every side of every cell
            let side = border_side_of(&bv)?;
            let mut m = BTreeMap::new();
            if let Some(s) = side {
                for k in ["left", "right", "top", "bottom"] {
                    m.insert(k.to_string(), s.clone());
                }
            }
            sp.borders = Some(m);
        } else {
            let mut m = BTreeMap::new();
            if has(&bv, "all")
                && let Some(s) = border_side_of(&get(&bv, "all"))?
            {
                for k in ["left", "right", "top", "bottom"] {
                    m.insert(k.to_string(), s.clone());
                }
            }
            for k in SIDES {
                if has(&bv, k)
                    && let Some(s) = border_side_of(&get(&bv, k))?
                {
                    m.insert(k.to_string(), s);
                }
            }
            sp.borders = Some(m);
            sp.outline = border_side_of(&get(&bv, "outline"))?;
            sp.inside = border_side_of(&get(&bv, "inside"))?;
            sp.inside_h = border_side_of(&get(&bv, "horizontal"))?;
            sp.inside_v = border_side_of(&get(&bv, "vertical"))?;
        }
    }
    let pr = get(x, "protection");
    if is_obj(&pr) && !nullish(&pr) {
        sp.protected = opt_b(&pr, "locked");
    }
    Ok(sp)
}

// ---- applying ----

type Sides = BTreeMap<String, BorderSide>;

impl StyleSpec {
    /// The border sides for the cell at (col,row) inside the range
    /// [c1..c2]x[r1..r2].
    fn sides_for(&self, col: u32, row: u32, c1: u32, r1: u32, c2: u32, r2: u32) -> Option<Sides> {
        if self.borders.is_none() && self.outline.is_none() && self.inside.is_none() && self.inside_h.is_none() && self.inside_v.is_none() {
            return None;
        }
        let mut m = self.borders.clone().unwrap_or_default();
        let mut set = |side: &str, b: &Option<BorderSide>| {
            if let Some(b) = b {
                m.insert(side.to_string(), b.clone());
            }
        };
        let h = self.inside_h.clone().or_else(|| self.inside.clone());
        let v = self.inside_v.clone().or_else(|| self.inside.clone());
        if row > r1 {
            set("top", &h);
        }
        if row < r2 {
            set("bottom", &h);
        }
        if col > c1 {
            set("left", &v);
        }
        if col < c2 {
            set("right", &v);
        }
        if row == r1 {
            set("top", &self.outline);
        }
        if row == r2 {
            set("bottom", &self.outline);
        }
        if col == c1 {
            set("left", &self.outline);
        }
        if col == c2 {
            set("right", &self.outline);
        }
        Some(m)
    }
}

fn argb(rgb: &str) -> String {
    format!("FF{rgb}")
}

/// The RRGGBB of an explicit color, None for theme/automatic colors.
fn color_rgb(c: &umya_spreadsheet::Color) -> Option<String> {
    let s = c.argb_str();
    if s.is_empty() || s == "00000000" {
        return None;
    }
    Some(if s.len() == 8 { s[2..].to_string() } else { s })
}

fn border_mut<'a>(st: &'a mut Style, side: &str) -> &'a mut Border {
    let b = st.borders_mut();
    match side {
        "left" => b.left_mut(),
        "right" => b.right_mut(),
        "top" => b.top_mut(),
        "bottom" => b.bottom_mut(),
        _ => b.diagonal_mut(),
    }
}

/// Merges sp (with the border sides of the cell) into st.
fn apply(st: &mut Style, sp: &StyleSpec, sides: Option<&Sides>, default_font: &(String, f64)) {
    if let Some(f) = &sp.font {
        let fresh = st.font().is_none();
        let font = st.font_mut();
        if fresh || font.name().is_empty() {
            font.set_name(default_font.0.clone());
        }
        if fresh || font.size() == 0.0 {
            font.set_size(default_font.1);
        }
        if let Some(n) = f.name.as_ref().filter(|n| !n.is_empty()) {
            font.set_name(n.clone());
            font.font_scheme_mut().set_val(umya_spreadsheet::structs::FontSchemeValues::None);
        }
        if let Some(s) = f.size {
            font.set_size(s);
        }
        if let Some(b) = f.bold {
            font.set_bold(b);
        }
        if let Some(i) = f.italic {
            font.set_italic(i);
        }
        if let Some(s) = f.strike {
            font.set_strikethrough(s);
        }
        if let Some(u) = &f.underline {
            let v = if u.is_empty() { UnderlineValues::None } else { UnderlineValues::from_str(u).unwrap_or(UnderlineValues::Single) };
            font.font_underline_mut().set_val(v);
        }
        if let Some(v) = &f.vert_align {
            if let Ok(x) = umya_spreadsheet::structs::VerticalAlignmentRunValues::from_str(v) {
                font.vertical_text_alignment_mut().set_val(x);
            }
        }
        if let Some(c) = &f.color {
            if c.is_empty() {
                *font.color_mut() = umya_spreadsheet::Color::default();
            } else {
                font.color_mut().set_argb_str(argb(c));
            }
        }
    } else if st.font().is_none() {
        // The cell keeps the default font explicitly (as excelize copies
        // style 0).
        let font = st.font_mut();
        font.set_name(default_font.0.clone());
        font.set_size(default_font.1);
    }
    if let Some(fill) = &sp.fill {
        if fill.is_empty() {
            st.remove_fill();
        } else {
            let pf = st.fill_mut().pattern_fill_mut();
            pf.set_pattern_type(PatternValues::Solid);
            pf.foreground_color_mut().set_argb_str(argb(fill));
        }
    }
    if let Some(a) = &sp.align {
        let al = st.alignment_mut();
        if let Some(h) = &a.horizontal {
            al.set_horizontal(HorizontalAlignmentValues::from_str(h).unwrap_or(HorizontalAlignmentValues::General));
        }
        if let Some(v) = &a.vertical {
            al.set_vertical(VerticalAlignmentValues::from_str(v).unwrap_or(VerticalAlignmentValues::Bottom));
        }
        if let Some(w) = a.wrap {
            al.set_wrap_text(w);
        }
        if let Some(i) = a.indent {
            al.set_indent(i.max(0) as u32);
        }
        if let Some(r) = a.rotation {
            al.set_text_rotation(if r < 0 { (90 - r) as u32 } else { r as u32 });
        }
    }
    if let Some(nf) = &sp.num_fmt {
        if nf.is_empty() || nf.eq_ignore_ascii_case("General") {
            st.remove_number_format();
        } else {
            let mut n = NumberingFormat::default();
            n.set_format_code(nf.clone());
            st.set_number_format(n);
        }
    }
    if let Some(id) = sp.num_fmt_id {
        // umya-spreadsheet panics on ids it does not know.
        let n = std::panic::catch_unwind(|| {
            let mut n = NumberingFormat::default();
            n.set_number_format_id(id);
            n
        });
        match n {
            Ok(n) => {
                st.set_number_format(n);
            }
            Err(_) => {
                if let Some(code) = xlsx::builtin_code(id) {
                    let mut n = NumberingFormat::default();
                    n.set_format_code(code);
                    st.set_number_format(n);
                }
            }
        }
    }
    if let Some(sides) = sides {
        for (side, s) in sides {
            let diag = side.starts_with("diagonal");
            if s.style == 0 {
                if diag {
                    if side == "diagonalUp" {
                        st.borders_mut().set_diagonal_up(false);
                    } else {
                        st.borders_mut().set_diagonal_down(false);
                    }
                    let b = st.borders_mut();
                    if !b.diagonal_up() && !b.diagonal_down() {
                        b.diagonal_mut().set_style(BorderStyleValues::None);
                    }
                } else {
                    *border_mut(st, side) = Border::default();
                }
                continue;
            }
            let bs = BorderStyleValues::from_str(BORDER_NAMES[s.style]).unwrap_or(BorderStyleValues::Thin);
            let b = border_mut(st, side);
            b.set_style(bs);
            let mut c = umya_spreadsheet::Color::default();
            c.set_argb_str(argb(&s.color));
            b.set_color(c);
            if side == "diagonalUp" {
                st.borders_mut().set_diagonal_up(true);
            } else if side == "diagonalDown" {
                st.borders_mut().set_diagonal_down(true);
            }
        }
    }
    if let Some(p) = sp.protected {
        st.protection_mut().set_locked(p);
    }
}

/// Accepts "A1", "A1:C3" (any corner order).
pub fn parse_range(r: &str) -> Result<(u32, u32, u32, u32)> {
    let up = r.trim().replace('$', "").to_uppercase();
    let parts: Vec<&str> = up.split(':').collect();
    if parts.len() > 2 || parts[0].is_empty() {
        return Err(err!("xlsx.invalidRange", up));
    }
    let (c1, r1) = parse_ref(parts[0]).map_err(|_| err!("xlsx.invalidRange", up))?;
    let (mut c2, mut r2) = (c1, r1);
    if parts.len() == 2 {
        (c2, r2) = parse_ref(parts[1]).map_err(|_| err!("xlsx.invalidRange", up))?;
    }
    Ok((c1.min(c2), r1.min(r2), c1.max(c2), r1.max(r2)))
}

fn has_style(st: &Style) -> bool {
    st.font().is_some() || st.fill().is_some() || st.borders().is_some() || st.alignment().is_some() || st.number_format().is_some() || st.protection().is_some()
}

/// Merges sp into every cell of the range.
pub fn apply_style(b: &mut Book, sheet: &str, r: &str, sp: &StyleSpec, replace: bool) -> Result<()> {
    let (c1, r1, c2, r2) = parse_range(r)?;
    let df = b.default_font.clone();
    let ws = b.sheet_mut(sheet)?;
    for row in r1..=r2 {
        for col in c1..=c2 {
            let sides = sp.sides_for(col, row, c1, r1, c2, r2);
            let c = ws.cell_mut((col, row));
            let mut st = if replace { Style::default() } else { c.style().clone() };
            apply(&mut st, sp, sides.as_ref(), &df);
            c.set_style(st);
        }
    }
    // Copy borders set on the edges of the range to the touching side of
    // already formatted neighbor cells, so the two cells do not disagree
    // about the line between them (as Excel's own UI does).
    let edges: [(&str, &str, i64, i64); 4] = [("top", "bottom", 0, -1), ("bottom", "top", 0, 1), ("left", "right", -1, 0), ("right", "left", 1, 0)];
    for (side, opposite, dc, dr) in edges {
        let cells: Vec<(u32, u32)> = match side {
            "top" => (c1..=c2).map(|c| (c, r1)).collect(),
            "bottom" => (c1..=c2).map(|c| (c, r2)).collect(),
            "left" => (r1..=r2).map(|r| (c1, r)).collect(),
            _ => (r1..=r2).map(|r| (c2, r)).collect(),
        };
        for (col, row) in cells {
            let Some(sides) = sp.sides_for(col, row, c1, r1, c2, r2) else { continue };
            let Some(s) = sides.get(side) else { continue };
            let (nc, nr) = (col as i64 + dc, row as i64 + dr);
            if nc < 1 || nr < 1 {
                continue;
            }
            let Some(nb) = ws.cell((nc as u32, nr as u32)) else { continue };
            if !has_style(nb.style()) {
                continue;
            }
            let mut st = nb.style().clone();
            let one: Sides = [(opposite.to_string(), s.clone())].into_iter().collect();
            apply(&mut st, &StyleSpec::default(), Some(&one), &df);
            ws.cell_mut((nc as u32, nr as u32)).set_style(st);
        }
    }
    Ok(())
}

/// Sets column widths from the displayed text of the used cells.
fn auto_fit(b: &mut Book, sheet: &str, c1: u32, c2: u32, min_w: f64, max_w: f64) -> Result<()> {
    let scale = b.default_font.1 / 11.0;
    let mut widths: BTreeMap<u32, f64> = BTreeMap::new();
    {
        let ws = b.sheet(sheet)?;
        for row in xlsx::rows(ws) {
            for (col, c) in row {
                if col < c1 || col > c2 {
                    continue;
                }
                let text = xlsx::cell_text(b, c);
                if text.is_empty() {
                    continue;
                }
                let w = text.split('\n').map(UnicodeWidthStr::width).max().unwrap_or(0);
                let fw = w as f64 * 1.1 * scale + 2.0;
                let e = widths.entry(col).or_insert(0.0);
                if fw > *e {
                    *e = fw;
                }
            }
        }
    }
    for (col, w) in widths {
        let w = min_w.max(max_w.min((w * 10.0).round() / 10.0));
        let name = column_name(col as i64)?;
        xlsx::set_col_width(b, sheet, &name, &name, w)?;
    }
    Ok(())
}

/// Builds an object from key/value pairs in order, skipping zero values.
fn kv<'js>(ctx: &rquickjs::Ctx<'js>, pairs: Vec<(&str, Option<Value<'js>>)>) -> R<'js> {
    let o = rquickjs::Object::new(ctx.clone())?;
    for (k, val) in pairs {
        let Some(val) = val else { continue };
        if let Some(s) = val.as_string() {
            let s = s.to_string().unwrap_or_default();
            if s.is_empty() || s == "#" {
                continue;
            }
        } else if val.as_bool() == Some(false) || val.as_number() == Some(0.0) || val.is_null() || val.is_undefined() {
            continue;
        } else if let Some(obj) = val.as_object()
            && obj.keys::<String>().count() == 0
        {
            continue;
        }
        o.set(k, val)?;
    }
    Ok(o.into_value())
}

fn style_object<'js>(ctx: &rquickjs::Ctx<'js>, st: &Style, df: &(String, f64)) -> R<'js> {
    let s = |x: String| Some(v(ctx, x));
    let font = match st.font() {
        Some(f) => {
            let color = color_rgb(f.color()).map(|c| format!("#{c}")).unwrap_or_default();
            let u = f.font_underline().val().value_string().to_string();
            let u = if u == "none" { String::new() } else { u };
            let va = f.vertical_text_alignment().val().value_string().to_string();
            let va = if va == "baseline" { String::new() } else { va };
            let name = if f.name().is_empty() { df.0.clone() } else { f.name().to_string() };
            let size = if f.size() == 0.0 { df.1 } else { f.size() };
            Some(kv(
                ctx,
                vec![
                    ("name", s(name)),
                    ("size", Some(v(ctx, size))),
                    ("bold", Some(v(ctx, f.bold()))),
                    ("italic", Some(v(ctx, f.italic()))),
                    ("underline", s(u)),
                    ("strike", Some(v(ctx, f.strikethrough()))),
                    ("color", s(color)),
                    ("vertAlign", s(va)),
                ],
            )?)
        }
        None => Some(kv(ctx, vec![("name", s(df.0.clone())), ("size", Some(v(ctx, df.1)))])?),
    };
    let border = match st.borders() {
        Some(b) => {
            let mut pairs = Vec::new();
            let mut side = |name: &'static str, bd: &Border| -> rquickjs::Result<()> {
                let style = bd.border_style().to_string();
                if style.is_empty() || style == "none" {
                    return Ok(());
                }
                let color = bd.color().as_ref().and_then(color_rgb).unwrap_or_else(|| "000000".into());
                pairs.push((name, Some(kv(ctx, vec![("style", s(style)), ("color", s(format!("#{color}")))])?)));
                Ok(())
            };
            side("left", b.left())?;
            side("right", b.right())?;
            side("top", b.top())?;
            side("bottom", b.bottom())?;
            if b.diagonal_up() {
                side("diagonalUp", b.diagonal())?;
            }
            if b.diagonal_down() {
                side("diagonalDown", b.diagonal())?;
            }
            Some(kv(ctx, pairs)?)
        }
        None => None,
    };
    let align = match st.alignment() {
        Some(a) => {
            let h = a.horizontal().value_string().to_string();
            let h = if h == "general" { String::new() } else { h };
            let vv = a.vertical().value_string().to_string();
            let vv = if vv == "bottom" { String::new() } else { vv };
            Some(kv(
                ctx,
                vec![
                    ("horizontal", s(h)),
                    ("vertical", s(vv)),
                    ("wrap", Some(v(ctx, a.wrap_text()))),
                    ("indent", Some(v(ctx, a.indent() as f64))),
                    ("rotation", Some(v(ctx, a.text_rotation() as f64))),
                ],
            )?)
        }
        None => None,
    };
    let mut fill = String::new();
    if let Some(f) = st.fill()
        && let Some(pf) = f.pattern_fill()
        && pf.pattern_type().value_string() == "solid"
        && let Some(c) = pf.foreground_color().and_then(color_rgb)
    {
        fill = format!("#{c}");
    }
    let num_fmt = match st.number_format() {
        Some(nf) if nf.number_format_id() != 0 && !nf.format_code().eq_ignore_ascii_case("General") => {
            if nf.format_code().is_empty() {
                Some(v(ctx, nf.number_format_id() as f64))
            } else {
                Some(v(ctx, nf.format_code().to_string()))
            }
        }
        _ => None,
    };
    kv(ctx, vec![("font", font), ("fill", s(fill)), ("border", border), ("alignment", align), ("numberFormat", num_fmt)])
}

pub(super) fn install(n: &N) {
    // xlsxStyle(book, sheet, range, spec, replace)
    n.set("xlsxStyle", |h, a| {
        let sp = parse_style(&a.arg(3)).map_err(|e| a.throw(&e))?;
        let (sheet, r, replace) = (a.str(1), a.str(2), a.bool(4));
        with_book(h, a, |b| apply_style(b, &sheet, &r, &sp, replace).map_err(|e| a.throw(&e)))?;
        a.undefined()
    });
    // xlsxGetStyle(book, sheet, cell) -> the cell's style in the same shape
    // that style() accepts (only non-default values are included).
    n.set("xlsxGetStyle", |h, a| {
        let (sheet, r) = (a.str(1), a.str(2).to_uppercase());
        with_book(h, a, |b| {
            let (col, row) = parse_ref(&r).map_err(|e| a.throw(&e))?;
            let ws = b.sheet(&sheet).map_err(|e| a.throw(&e))?;
            let st = ws.cell((col, row)).map(|c| c.style().clone()).unwrap_or_default();
            style_object(&a.ctx, &st, &b.default_font)
        })
    });
    n.set("xlsxMerge", |h, a| {
        let (sheet, r, unmerge) = (a.str(1), a.str(2), a.bool(3));
        with_book(h, a, |b| {
            let (c1, r1, c2, r2) = parse_range(&r).map_err(|e| a.throw(&e))?;
            let range = format!("{}:{}", cell(c1, r1), cell(c2, r2));
            let ws = b.sheet_mut(&sheet).map_err(|e| a.throw(&e))?;
            ws.merge_cells_mut().retain(|m| {
                let (x1, y1, x2, y2) = parse_range(&m.range()).unwrap_or((0, 0, 0, 0));
                // Drop merges that overlap the new one (or the one removed).
                x2 < c1 || x1 > c2 || y2 < r1 || y1 > r2
            });
            if !unmerge {
                ws.add_merge_cells(range);
            }
            Ok(())
        })?;
        a.undefined()
    });
    // xlsxFreeze(book, sheet, topLeftCell) — "" removes the panes.
    n.set("xlsxFreeze", |h, a| {
        let (sheet, r) = (a.str(1), a.str(2).to_uppercase());
        with_book(h, a, |b| freeze(b, &sheet, &r).map_err(|e| a.throw(&e)))?;
        a.undefined()
    });
    n.set("xlsxAutoFilter", |h, a| {
        let (sheet, r) = (a.str(1), a.str(2));
        with_book(h, a, |b| {
            let (c1, r1, c2, r2) = parse_range(&r).map_err(|e| a.throw(&e))?;
            let ws = b.sheet_mut(&sheet).map_err(|e| a.throw(&e))?;
            ws.set_auto_filter(format!("{}:{}", cell(c1, r1), cell(c2, r2)));
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxSetRowHeight", |h, a| {
        let (sheet, row, ht) = (a.str(1), a.int(2), a.f64(3));
        with_book(h, a, |b| {
            if !(1..=1_048_576).contains(&row) {
                return Err(a.throw(&Error::msg(format!("invalid row number {row}"))));
            }
            if !(0.0..=409.0).contains(&ht) {
                return Err(a.throw(&Error::msg("the height of the row must be less than or equal to 409 points")));
            }
            let ws = b.sheet_mut(&sheet).map_err(|e| a.throw(&e))?;
            let rd = ws.row_dimension_mut(row as u32);
            rd.set_height(ht);
            rd.set_custom_height(true);
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxGetColWidth", |h, a| {
        let (sheet, col) = (a.str(1), a.str(2).to_uppercase());
        let w = with_book(h, a, |b| xlsx::col_width(b, &sheet, &col).map_err(|e| a.throw(&e)))?;
        a.ret(w)
    });
    // xlsxAutoFit(book, sheet, firstCol, lastCol, min, max)
    n.set("xlsxAutoFit", |h, a| {
        let (sheet, c1, c2, mn, mx) = (a.str(1), a.int(2), a.int(3), a.f64(4), a.f64(5));
        with_book(h, a, |b| auto_fit(b, &sheet, c1.max(1) as u32, c2.max(1) as u32, mn, mx).map_err(|e| a.throw(&e)))?;
        a.undefined()
    });
    n.set("xlsxDefaultFont", |h, a| {
        let (name, size) = with_book(h, a, |b| Ok(b.default_font.clone()))?;
        object(&a.ctx, vec![("name", v(&a.ctx, name)), ("size", v(&a.ctx, size))])
    });
    let _ = js::nullish;
}

fn freeze(b: &mut Book, sheet: &str, r: &str) -> Result<()> {
    use umya_spreadsheet::structs::{Pane, PaneStateValues, PaneValues, SheetView};
    let ws = b.sheet_mut(sheet)?;
    let views = ws.sheet_views_mut().sheet_view_list_mut();
    if views.is_empty() {
        views.push(SheetView::default());
    }
    let old = views[0].clone();
    let mut view = SheetView::default();
    view.set_tab_selected(old.tab_selected());
    view.set_workbook_view_id(old.workbook_view_id());
    view.set_show_grid_lines(old.show_grid_lines());
    if old.zoom_scale() != 0 {
        view.set_zoom_scale(old.zoom_scale());
    }
    if r.is_empty() || r == "A1" {
        views[0] = view;
        return Ok(());
    }
    let (col, row) = parse_ref(r).map_err(|_| err!("xlsx.invalidRange", r))?;
    let pane_kind = if col == 1 {
        PaneValues::BottomLeft
    } else if row == 1 {
        PaneValues::TopRight
    } else {
        PaneValues::BottomRight
    };
    let mut pane = Pane::default();
    pane.set_horizontal_split((col - 1) as f64);
    pane.set_vertical_split((row - 1) as f64);
    let mut tl = umya_spreadsheet::structs::Coordinate::default();
    tl.set_coordinate(r);
    pane.set_top_left_cell(tl);
    pane.set_active_pane(pane_kind);
    pane.set_state(PaneStateValues::Frozen);
    view.set_pane(pane);
    views[0] = view;
    Ok(())
}
