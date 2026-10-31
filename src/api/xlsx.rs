//! Workbooks (xlsx) on top of umya-spreadsheet.

use std::io::{Cursor, Read, Write};
use std::sync::LazyLock;

use regex::Regex;
use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, JsLifetime, Value};
use umya_spreadsheet::{CellRawValue, NumberingFormat, Workbook, Worksheet};

use super::js::{self, elems, nullish};
use super::{A, Host, N, R};
use crate::{Error, Result, err, i18n};

/// The handle of an open workbook in JavaScript.
#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename = "Workbook")]
pub struct Handle {
    #[qjs(skip_trace)]
    pub id: u64,
}

/// An open workbook.
pub struct Book {
    pub wb: Workbook,
    pub path: String,
    pub date1904: bool,
    /// A workbook from create() whose default sheet has not been used yet;
    /// the first addSheet renames it.
    pub fresh: bool,
    pub formulas: bool,
    pub closed: bool,
    /// The font of the Normal style (name, size).
    pub default_font: (String, f64),
    /// Whether the default font must be written into styles.xml on save.
    pub font_changed: bool,
}

pub(crate) fn zerr(e: impl std::fmt::Display) -> Error {
    let s = e.to_string();
    if s.contains("InvalidArchive") || s.contains("invalid Zip archive") || s.contains("Could not find EOCD") || s.contains("zip") {
        return Error::msg("zip: not a valid zip file");
    }
    Error::msg(s)
}

impl Book {
    fn new(wb: Workbook, path: String, styles_xml: Option<&[u8]>) -> Book {
        let default_font = styles_xml.and_then(first_font).unwrap_or(("Calibri".into(), 11.0));
        let date1904 = false; // see read_date1904
        Book { wb, path, date1904, fresh: false, formulas: false, closed: false, default_font, font_changed: false }
    }

    pub fn sheet(&self, name: &str) -> Result<&Worksheet> {
        self.wb.sheet_by_name(name).map_err(|_| Error::msg(format!("sheet {name} does not exist")))
    }

    pub fn sheet_mut(&mut self, name: &str) -> Result<&mut Worksheet> {
        if self.fresh && name == "Sheet1" {
            self.fresh = false;
        }
        self.wb.sheet_by_name_mut(name).map_err(|_| Error::msg(format!("sheet {name} does not exist")))
    }

    /// The workbook as xlsx bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        umya_spreadsheet::writer::xlsx::write_writer(&self.wb, &mut buf).map_err(zerr)?;
        let data = buf.into_inner();
        if !self.formulas && !self.font_changed {
            return Ok(data);
        }
        self.postprocess(&data)
    }

    /// Writes what umya-spreadsheet cannot: the default font of the Normal
    /// style and the "recalculate on load" flag.
    fn postprocess(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut zr = zip::ZipArchive::new(Cursor::new(data)).map_err(zerr)?;
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..zr.len() {
            let mut f = zr.by_index(i).map_err(zerr)?;
            let name = f.name().to_string();
            let mut content = Vec::new();
            f.read_to_end(&mut content).map_err(zerr)?;
            if name == "xl/workbook.xml" && self.formulas {
                let s = String::from_utf8_lossy(&content);
                content = CALC_PR.replace(&s, |c: &regex::Captures| {
                    if c[0].contains("fullCalcOnLoad") { c[0].to_string() } else { format!("{} fullCalcOnLoad=\"1\"{}", &c[1], &c[2]) }
                }).into_owned().into_bytes();
            }
            if name == "xl/styles.xml" && self.font_changed {
                content = set_first_font(&String::from_utf8_lossy(&content), &self.default_font.0, self.default_font.1).into_bytes();
            }
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zw.start_file(name, opts).map_err(zerr)?;
            zw.write_all(&content).map_err(zerr)?;
        }
        Ok(zw.finish().map_err(zerr)?.into_inner())
    }
}

static CALC_PR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(<calcPr\b[^>]*?)(\s*/?>)").unwrap());
static FONTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<fonts\b[^>]*>\s*<font\b[^>]*>(.*?)</font>").unwrap());
static FONT_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<name\s+val="([^"]*)"\s*/>"#).unwrap());
static FONT_SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<sz\s+val="([^"]*)"\s*/>"#).unwrap());

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The first font of styles.xml (the Normal style's).
fn first_font(styles: &[u8]) -> Option<(String, f64)> {
    let s = String::from_utf8_lossy(styles);
    let c = FONTS.captures(&s)?;
    let body = &c[1];
    let name = FONT_NAME.captures(body).map(|m| xml_unescape(&m[1]))?;
    let size = FONT_SIZE.captures(body).and_then(|m| m[1].parse().ok()).unwrap_or(11.0);
    Some((name, size))
}

fn set_first_font(styles: &str, name: &str, size: f64) -> String {
    let Some(m) = FONTS.captures(styles).and_then(|c| c.get(1)) else { return styles.to_string() };
    let body = m.as_str();
    let mut nb = FONT_NAME.replace(body, format!("<name val=\"{}\"/>", xml_escape(name)).as_str()).into_owned();
    nb = FONT_SIZE.replace(&nb, format!("<sz val=\"{}\"/>", fmt_num(size)).as_str()).into_owned();
    // The theme's font would win over the name.
    nb = Regex::new(r#"<scheme\s+val="[^"]*"\s*/>"#).unwrap().replace(&nb, "").into_owned();
    format!("{}{}{}", &styles[..m.start()], nb, &styles[m.end()..])
}

/// Reads the bytes of a part of an xlsx file.
fn zip_part(data: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zr = zip::ZipArchive::new(Cursor::new(data)).ok()?;
    let mut f = zr.by_name(name).ok()?;
    let mut b = Vec::new();
    f.read_to_end(&mut b).ok()?;
    Some(b)
}

fn read_date1904(data: &[u8]) -> bool {
    zip_part(data, "xl/workbook.xml")
        .map(|b| {
            let s = String::from_utf8_lossy(&b);
            Regex::new(r#"<workbookPr\b[^>]*\bdate1904="(1|true)""#).unwrap().is_match(&s)
        })
        .unwrap_or(false)
}

/// Opens xlsx bytes.
pub fn open_bytes(data: &[u8], path: String) -> Result<Book> {
    if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err(Error::msg("the encrypted file is not supported"));
    }
    let wb = umya_spreadsheet::reader::xlsx::read_reader(Cursor::new(data), true).map_err(zerr)?;
    let styles = zip_part(data, "xl/styles.xml");
    let mut b = Book::new(wb, path, styles.as_deref());
    b.date1904 = read_date1904(data);
    Ok(b)
}

// ---- cell names ----

/// "A" for 1, "AB" for 28.
pub fn column_name(mut n: i64) -> Result<String> {
    if !(1..=16384).contains(&n) {
        return Err(Error::msg(format!("the column number must be greater than or equal to 1 and less than or equal to 16384, got {n}")));
    }
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    s.reverse();
    Ok(String::from_utf8(s).unwrap())
}

pub fn column_number(name: &str) -> Result<u32> {
    let up = name.trim().to_uppercase();
    if up.is_empty() || !up.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(Error::msg(format!("invalid column name \"{name}\"")));
    }
    let mut n: u64 = 0;
    for b in up.bytes() {
        n = n * 26 + (b - b'A' + 1) as u64;
        if n > 16384 {
            return Err(Error::msg(format!("invalid column name \"{name}\"")));
        }
    }
    Ok(n as u32)
}

pub fn cell_name(col: u32, row: u32) -> Result<String> {
    if row < 1 || row > 1_048_576 {
        return Err(Error::msg(format!("invalid cell reference [{col}, {row}]")));
    }
    Ok(format!("{}{row}", column_name(col as i64)?))
}

pub(crate) fn cell(col: u32, row: u32) -> String {
    cell_name(col, row).unwrap_or_default()
}

/// The (column, row) of "B3" (1-based).
pub fn parse_ref(r: &str) -> Result<(u32, u32)> {
    let bad = || Error::msg(format!("cannot convert cell \"{r}\" to coordinates: invalid cell name \"{r}\""));
    let s = r.replace('$', "");
    let i = s.find(|c: char| c.is_ascii_digit()).ok_or_else(bad)?;
    let (c, rr) = s.split_at(i);
    if c.is_empty() || !c.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(bad());
    }
    let col = column_number(c).map_err(|_| bad())?;
    let row: u32 = rr.parse().map_err(|_| bad())?;
    if row < 1 || row > 1_048_576 {
        return Err(bad());
    }
    Ok((col, row))
}

// ---- dates ----

/// The Excel serial number of a wall-clock time.
pub fn to_serial(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32, date1904: bool) -> f64 {
    let date = chrono::NaiveDate::from_ymd_opt(y, mo, d).unwrap_or_default();
    let base = if date1904 { chrono::NaiveDate::from_ymd_opt(1904, 1, 1).unwrap() } else { chrono::NaiveDate::from_ymd_opt(1899, 12, 30).unwrap() };
    let mut days = (date - base).num_days() as f64;
    if !date1904 && days < 61.0 {
        days -= 1.0; // Excel's 1900-02-29
    }
    days + (h as f64 * 3600.0 + mi as f64 * 60.0 + s as f64 + ms as f64 / 1000.0) / 86400.0
}

/// The wall-clock time of an Excel serial number, rounded to the
/// millisecond: (y, month, d, h, mi, s, ms).
pub fn from_serial(f: f64, date1904: bool) -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
    if !f.is_finite() || f < 0.0 || f > 2_958_466.0 {
        return None;
    }
    let mut days = f.floor();
    let mut ms = ((f - days) * 86_400_000.0).round() as i64;
    if ms >= 86_400_000 {
        days += 1.0;
        ms -= 86_400_000;
    }
    let base = if date1904 {
        chrono::NaiveDate::from_ymd_opt(1904, 1, 1).unwrap()
    } else if days < 61.0 {
        if days == 60.0 {
            days = 59.0; // 1900-02-29 does not exist
        }
        chrono::NaiveDate::from_ymd_opt(1899, 12, 31).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(1899, 12, 30).unwrap()
    };
    let date = base + chrono::Duration::days(days as i64);
    use chrono::Datelike;
    Some((
        date.year(),
        date.month(),
        date.day(),
        (ms / 3_600_000) as u32,
        (ms / 60_000 % 60) as u32,
        (ms / 1000 % 60) as u32,
        (ms % 1000) as u32,
    ))
}

/// Built-in number format ids that display dates or times, including the
/// Japanese locale specific ones.
pub fn builtin_date_fmt(id: u32) -> bool {
    matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58)
}

/// Whether a custom number format displays a date or time. Literal text,
/// escapes and bracketed sections are ignored; elapsed time formats
/// ([h]:mm) are treated as plain numbers.
pub fn date_format_code(code: &str) -> bool {
    let code = code.split(';').next().unwrap_or("");
    let lower = code.to_lowercase();
    if lower.contains("[h") || lower.contains("[m") || lower.contains("[s") {
        return false;
    }
    let b = lower.as_bytes();
    let mut out = String::new();
    let mut in_quote = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_quote {
            if c == b'"' {
                in_quote = false;
            }
        } else if c == b'"' {
            in_quote = true;
        } else if c == b'\\' || c == b'_' || c == b'*' {
            i += 1;
        } else if c == b'[' {
            if let Some(j) = lower[i..].find(']') {
                i += j;
            }
        } else {
            out.push(c as char);
        }
        i += 1;
    }
    out.replace("general", "").contains(['y', 'm', 'd', 'h', 's'])
}

/// Whether a style's number format displays dates.
pub(crate) fn style_is_date(nf: Option<&NumberingFormat>) -> bool {
    let Some(nf) = nf else { return false };
    let id = nf.number_format_id();
    if id < 164 && builtin_code(id).is_some() {
        return builtin_date_fmt(id) || date_format_code(nf.format_code());
    }
    date_format_code(nf.format_code())
}

/// The code of a built-in number format (as Japanese Excel shows them).
pub fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "yyyy/m/d",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "yyyy/m/d h:mm",
        27..=36 | 50..=58 => "yyyy/m/d",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// The code a number format displays with.
pub(crate) fn format_code(nf: Option<&NumberingFormat>) -> String {
    let Some(nf) = nf else { return "General".into() };
    let id = nf.number_format_id();
    if id < 164
        && let Some(c) = builtin_code(id)
    {
        return c.to_string();
    }
    let c = nf.format_code();
    if c.is_empty() { "General".into() } else { c.to_string() }
}

// ---- values ----

/// A number as Go's strconv.FormatFloat(f, 'f', -1, 64) would show it in
/// the General format (no exponent for usual magnitudes).
pub fn fmt_num(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f == f.trunc() && f.abs() < 1e21 {
        return format!("{}", f as i128);
    }
    let s = format!("{f}");
    s
}

/// The text a cell displays (its value formatted with its number format).
pub(crate) fn cell_text(b: &Book, c: &umya_spreadsheet::Cell) -> String {
    match c.raw_value() {
        CellRawValue::Empty => String::new(),
        CellRawValue::Bool(v) => if *v { "TRUE".into() } else { "FALSE".into() },
        CellRawValue::Numeric(f) => {
            let nf = c.style().number_format();
            let code = format_code(nf);
            if style_is_date(nf) {
                if let Some(t) = from_serial(*f, b.date1904) {
                    return super::numfmt::format_date(t, &code);
                }
            }
            super::numfmt::format_number(*f, &code)
        }
        other => other.to_string(),
    }
}

/// The value of a cell as JavaScript.
pub(crate) fn cell_value<'js>(ctx: &Ctx<'js>, b: &Book, c: Option<&umya_spreadsheet::Cell>) -> R<'js> {
    let Some(c) = c else { return Ok(Value::new_null(ctx.clone())) };
    match c.raw_value() {
        CellRawValue::Empty => Ok(Value::new_null(ctx.clone())),
        CellRawValue::Bool(v) => Ok(Value::new_bool(ctx.clone(), *v)),
        CellRawValue::Numeric(f) => {
            if style_is_date(c.style().number_format())
                && let Some((y, mo, d, h, mi, s, ms)) = from_serial(*f, b.date1904)
            {
                return js::date_local(ctx, y, mo, d, h, mi, s, ms);
            }
            Ok(Value::new_number(ctx.clone(), *f))
        }
        CellRawValue::String(s) | CellRawValue::Lazy(s) => {
            if s.is_empty() {
                return Ok(Value::new_null(ctx.clone()));
            }
            Ok(js::v(ctx, s.to_string()))
        }
        other => {
            let s = other.to_string();
            if s.is_empty() { Ok(Value::new_null(ctx.clone())) } else { Ok(js::v(ctx, s)) }
        }
    }
}

fn has_value(c: &umya_spreadsheet::Cell) -> bool {
    match c.raw_value() {
        CellRawValue::Empty => false,
        CellRawValue::String(s) | CellRawValue::Lazy(s) => !s.is_empty(),
        _ => true,
    }
}

/// The used cells of a sheet as rows (like excelize's GetRows): every row
/// up to the last one with a value, each up to its last value.
pub(crate) fn rows(ws: &Worksheet) -> Vec<Vec<(u32, &umya_spreadsheet::Cell)>> {
    let mut by_row: std::collections::BTreeMap<u32, Vec<(u32, &umya_spreadsheet::Cell)>> = Default::default();
    for c in ws.cells() {
        if has_value(c) {
            let co = c.coordinate();
            by_row.entry(co.row_num()).or_default().push((co.col_num(), c));
        }
    }
    let last = by_row.keys().next_back().copied().unwrap_or(0);
    let mut out = Vec::with_capacity(last as usize);
    for r in 1..=last {
        let mut v = by_row.remove(&r).unwrap_or_default();
        v.sort_by_key(|(c, _)| *c);
        out.push(v);
    }
    out
}

fn date_style(code: &str) -> NumberingFormat {
    let mut nf = NumberingFormat::default();
    nf.set_format_code(code);
    nf
}

/// Writes a JavaScript value into a cell.
pub(crate) fn set_value(b: &mut Book, sheet: &str, r: &str, v: &Value) -> Result<()> {
    let (col, row) = parse_ref(r)?;
    let date1904 = b.date1904;
    let ws = b.sheet_mut(sheet)?;
    let c = ws.cell_mut((col, row));
    if nullish(v) {
        c.cell_value_mut().set_blank();
        return Ok(());
    }
    if js::is_date(v) {
        let Some((y, mo, d, h, mi, s, ms)) = js::date_fields(v) else {
            c.cell_value_mut().set_blank();
            return Ok(());
        };
        c.cell_value_mut().remove_formula();
        c.set_value_number(to_serial(y, mo, d, h, mi, s, ms, date1904));
        if !style_is_date(c.style().number_format()) {
            let code = if h == 0 && mi == 0 && s == 0 && ms == 0 { "yyyy/m/d" } else { "yyyy/m/d h:mm" };
            c.style_mut().set_number_format(date_style(code));
        }
        return Ok(());
    }
    c.cell_value_mut().remove_formula();
    if let Some(f) = v.as_number() {
        if f.is_finite() {
            c.set_value_number(f);
        } else {
            c.cell_value_mut().set_blank();
        }
    } else if let Some(x) = v.as_bool() {
        c.set_value_bool(x);
    } else {
        c.set_value_string(js::to_str(v));
    }
    Ok(())
}

// ---- natives ----

pub(crate) fn handle_id(v: &Value) -> Option<u64> {
    let o = v.as_object()?;
    let c = Class::<Handle>::from_object(o)?;
    Some(c.borrow().id)
}

/// Runs f with the open workbook of the handle.
pub(crate) fn with_book<'js, T>(h: &Host, a: &A<'js>, f: impl FnOnce(&mut Book) -> rquickjs::Result<T>) -> rquickjs::Result<T> {
    let Some(id) = handle_id(&a.arg(0)) else { return Err(a.throw(&err!("xlsx.invalidHandle"))) };
    let mut books = h.books.borrow_mut();
    let Some(b) = books.get_mut(&id) else { return Err(a.throw(&err!("xlsx.invalidHandle"))) };
    if b.closed {
        return Err(a.throw(&err!("xlsx.closed")));
    }
    f(b)
}

fn track<'js>(h: &Host, ctx: &Ctx<'js>, b: Book) -> R<'js> {
    let id = h.next_id();
    h.books.borrow_mut().insert(id, b);
    Ok(Class::instance(ctx.clone(), Handle { id })?.into_value())
}

fn upper(s: String) -> String {
    s.to_uppercase()
}

/// Checks a new sheet name like Excel (and excelize) do.
fn check_sheet_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::msg("the sheet name can not be blank"));
    }
    if name.chars().count() > 31 {
        return Err(Error::msg("the sheet name length exceeds the 31 characters limit"));
    }
    if name.contains([':', '\\', '/', '?', '*', '[', ']']) {
        return Err(Error::msg("the sheet can not contain any of the characters :\\/?*[or]"));
    }
    Ok(())
}

pub(super) fn install(n: &N) {
    n.set("xlsxOpen", |h, a| {
        let path = a.str(0);
        let data = std::fs::read(&path).map_err(|e| a.throw(&Error::path("open", path.clone(), e)))?;
        let b = open_bytes(&data, path.clone()).map_err(|e| super::throw_path(&a.ctx, &path, e))?;
        track(h, &a.ctx, b)
    });
    n.set("xlsxOpenBytes", |h, a| {
        let b = open_bytes(&a.bytes(0)?, a.str(1)).map_err(|e| a.throw(&e))?;
        track(h, &a.ctx, b)
    });
    n.set("xlsxCreate", |h, a| {
        let mut b = Book::new(umya_spreadsheet::new_file(), String::new(), None);
        b.fresh = true;
        // New workbooks use the language's default font (游ゴシック for
        // Japanese, like Excel itself); create({ font }) overrides it.
        let mut font = i18n::current().excel.font.clone();
        let o = a.arg(0);
        if o.is_object() && js::has(&o, "font") {
            let f = js::get(&o, "font");
            font = if f.is_null() { String::new() } else { js::to_str(&f) };
        }
        if !font.is_empty() {
            b.default_font.0 = font;
            b.font_changed = true;
        }
        track(h, &a.ctx, b)
    });
    n.set("xlsxPath", |h, a| {
        let p = with_book(h, a, |b| Ok(b.path.clone()))?;
        if p.is_empty() { a.null() } else { a.ret(p) }
    });
    n.set("xlsxSheetNames", |h, a| {
        let names = with_book(h, a, |b| Ok(b.wb.sheet_collection().iter().map(|s| s.name().to_string()).collect::<Vec<_>>()))?;
        a.ret(names)
    });
    n.set("xlsxAddSheet", |h, a| {
        let name = a.str(1);
        with_book(h, a, |b| {
            if b.wb.sheet_by_name(&name).is_ok() {
                return Err(a.throw(&err!("xlsx.sheetExists", name)));
            }
            check_sheet_name(&name).map_err(|e| a.throw(&e))?;
            if b.fresh {
                b.fresh = false;
                b.wb.set_sheet_name(0, name.clone()).map_err(|e| a.throw(&Error::msg(e.to_string())))?;
                return Ok(());
            }
            b.wb.new_sheet(name.clone()).map_err(|e| a.throw(&Error::msg(e.to_string())))?;
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxRemoveSheet", |h, a| {
        let name = a.str(1);
        with_book(h, a, |b| {
            if b.wb.sheet_by_name(&name).is_err() {
                return Err(a.throw(&Error::msg(format!("sheet {name} does not exist"))));
            }
            b.wb.remove_sheet_by_name(&name).map_err(|e| a.throw(&Error::msg(e.to_string())))?;
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxRenameSheet", |h, a| {
        let (from, to) = (a.str(1), a.str(2));
        with_book(h, a, |b| {
            let idx = b.wb.sheet_collection().iter().position(|s| s.name() == from);
            let Some(idx) = idx else { return Err(a.throw(&Error::msg(format!("sheet {from} does not exist")))) };
            if from == to {
                return Ok(());
            }
            check_sheet_name(&to).map_err(|e| a.throw(&e))?;
            if b.wb.sheet_by_name(&to).is_ok() {
                return Err(a.throw(&err!("xlsx.sheetExists", to)));
            }
            b.wb.set_sheet_name(idx, to.clone()).map_err(|e| a.throw(&Error::msg(e.to_string())))?;
            if b.fresh && from == "Sheet1" {
                b.fresh = false;
            }
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxHasSheet", |h, a| {
        let name = a.str(1);
        a.ret(with_book(h, a, |b| Ok(b.wb.sheet_by_name(&name).is_ok()))?)
    });
    n.set("xlsxGet", |h, a| {
        let (sheet, r) = (a.str(1), upper(a.str(2)));
        with_book(h, a, |b| {
            let (col, row) = parse_ref(&r).map_err(|e| a.throw(&e))?;
            let ws = b.sheet(&sheet).map_err(|e| a.throw(&e))?;
            cell_value(&a.ctx, b, ws.cell((col, row)))
        })
    });
    n.set("xlsxGetText", |h, a| {
        let (sheet, r) = (a.str(1), upper(a.str(2)));
        let s = with_book(h, a, |b| {
            let (col, row) = parse_ref(&r).map_err(|e| a.throw(&e))?;
            let ws = b.sheet(&sheet).map_err(|e| a.throw(&e))?;
            Ok(ws.cell((col, row)).map(|c| cell_text(b, c)).unwrap_or_default())
        })?;
        a.ret(s)
    });
    n.set("xlsxSet", |h, a| {
        let (sheet, r, v) = (a.str(1), upper(a.str(2)), a.arg(3));
        with_book(h, a, |b| set_value(b, &sheet, &r, &v).map_err(|e| a.throw(&e)))?;
        a.undefined()
    });
    n.set("xlsxGetFormula", |h, a| {
        let (sheet, r) = (a.str(1), upper(a.str(2)));
        let f = with_book(h, a, |b| {
            let (col, row) = parse_ref(&r).map_err(|e| a.throw(&e))?;
            let ws = b.sheet(&sheet).map_err(|e| a.throw(&e))?;
            Ok(ws.cell((col, row)).map(|c| c.formula().to_string()).unwrap_or_default())
        })?;
        if f.is_empty() {
            return a.null();
        }
        a.ret(if f.starts_with('=') { f } else { format!("={f}") })
    });
    n.set("xlsxSetFormula", |h, a| {
        let (sheet, r) = (a.str(1), upper(a.str(2)));
        let formula = a.str(3);
        let formula = formula.strip_prefix('=').unwrap_or(&formula).to_string();
        with_book(h, a, |b| {
            let (col, row) = parse_ref(&r).map_err(|e| a.throw(&e))?;
            let ws = b.sheet_mut(&sheet).map_err(|e| a.throw(&e))?;
            let c = ws.cell_mut((col, row));
            c.cell_value_mut().set_blank();
            c.set_formula(formula.clone());
            b.formulas = true;
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxCalc", |h, a| {
        let (sheet, r) = (a.str(1), upper(a.str(2)));
        let s = with_book(h, a, |b| super::xlsx_calc::calc_cell(b, &sheet, &r).map_err(|e| a.throw(&e)))?;
        a.ret(s)
    });
    // xlsxReadAll(book, sheet, text) -> any[][]
    n.set("xlsxReadAll", |h, a| {
        let sheet = a.str(1);
        let text = a.bool(2);
        with_book(h, a, |b| {
            let ws = b.sheet(&sheet).map_err(|e| a.throw(&e))?;
            let mut out = Vec::new();
            for row in rows(ws) {
                let width = row.last().map(|(c, _)| *c).unwrap_or(0) as usize;
                let mut vals: Vec<Value> = Vec::with_capacity(width);
                let mut it = row.into_iter().peekable();
                for col in 1..=width as u32 {
                    let cell = match it.peek() {
                        Some((c, _)) if *c == col => it.next().map(|(_, c)| c),
                        _ => None,
                    };
                    vals.push(if text {
                        js::v(&a.ctx, cell.map(|c| cell_text(b, c)).unwrap_or_default())
                    } else {
                        cell_value(&a.ctx, b, cell)?
                    });
                }
                out.push(js::array(&a.ctx, vals)?);
            }
            js::array(&a.ctx, out)
        })
    });
    // xlsxWriteRows(book, sheet, startRef, rows: any[][])
    n.set("xlsxWriteRows", |h, a| {
        let sheet = a.str(1);
        let start = upper(a.opt_str(2, "A1"));
        let data = a.arg(3);
        with_book(h, a, |b| {
            let (c0, r0) = parse_ref(&start).map_err(|e| a.throw(&e))?;
            b.sheet(&sheet).map_err(|e| a.throw(&e))?;
            for (r, rowv) in elems(&data).iter().enumerate() {
                for (c, v) in elems(rowv).iter().enumerate() {
                    if nullish(v) {
                        continue;
                    }
                    let name = cell_name(c0 + c as u32, r0 + r as u32).map_err(|e| a.throw(&e))?;
                    set_value(b, &sheet, &name, v).map_err(|e| a.throw(&e.context(format!("{sheet}!{name}"))))?;
                }
            }
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxRowCount", |h, a| {
        let sheet = a.str(1);
        let n = with_book(h, a, |b| Ok(rows(b.sheet(&sheet).map_err(|e| a.throw(&e))?).len()))?;
        a.ret(n as f64)
    });
    n.set("xlsxDimension", |h, a| {
        let sheet = a.str(1);
        let d = with_book(h, a, |b| {
            let rs = rows(b.sheet(&sheet).map_err(|e| a.throw(&e))?);
            let max_col = rs.iter().filter_map(|r| r.last().map(|(c, _)| *c)).max().unwrap_or(0);
            if rs.is_empty() || max_col == 0 {
                return Ok(String::new());
            }
            Ok(format!("A1:{}", cell(max_col, rs.len() as u32)))
        })?;
        a.ret(d)
    });
    n.set("xlsxSetColWidth", |h, a| {
        let (sheet, c1, c2, w) = (a.str(1), a.str(2), a.str(3), a.f64(4));
        with_book(h, a, |b| set_col_width(b, &sheet, &c1, &c2, w).map_err(|e| a.throw(&e)))?;
        a.undefined()
    });
    n.set("xlsxSave", |h, a| {
        let mut path = a.str(1);
        with_book(h, a, |b| {
            if path.is_empty() {
                path = b.path.clone();
            }
            if path.is_empty() {
                return Err(a.throw(&err!("xlsx.noDestination")));
            }
            let data = b.to_bytes().map_err(|e| a.throw(&e))?;
            std::fs::write(&path, data).map_err(|e| a.throw(&Error::path("open", path.clone(), e)))?;
            b.path = path.clone();
            Ok(())
        })?;
        a.undefined()
    });
    n.set("xlsxToBytes", |h, a| {
        let data = with_book(h, a, |b| b.to_bytes().map_err(|e| a.throw(&e)))?;
        js::array_buffer(&a.ctx, data)
    });
    n.set("xlsxClose", |h, a| {
        if let Some(id) = handle_id(&a.arg(0)) {
            h.books.borrow_mut().remove(&id);
            // A closed handle keeps reporting "closed".
            h.books.borrow_mut().insert(id, Book { closed: true, ..Book::new(Workbook::default(), String::new(), None) });
        }
        a.undefined()
    });
    n.set("xlsxCellName", |_, a| {
        let s = cell_name(a.int(0).max(0) as u32, a.int(1).max(0) as u32).map_err(|e| a.throw(&e))?;
        a.ret(s)
    });
    n.set("xlsxParseRef", |_, a| {
        let (c, r) = parse_ref(&upper(a.str(0))).map_err(|e| a.throw(&e))?;
        a.ret(vec![c as f64, r as f64])
    });
    n.set("xlsxColumnName", |_, a| {
        let s = column_name(a.int(0)).map_err(|e| a.throw(&e))?;
        a.ret(s)
    });
}

pub(crate) fn set_col_width(b: &mut Book, sheet: &str, c1: &str, c2: &str, w: f64) -> Result<()> {
    let (mut x, mut y) = (column_number(c1)?, column_number(c2)?);
    if y < x {
        std::mem::swap(&mut x, &mut y);
    }
    if !(0.0..=255.0).contains(&w) {
        return Err(Error::msg("the width of the column must be less than or equal to 255 characters"));
    }
    let ws = b.sheet_mut(sheet)?;
    for c in x..=y {
        let name = column_name(c as i64)?;
        ws.column_dimension_mut(&name).set_width(w);
    }
    Ok(())
}

pub(crate) fn col_width(b: &Book, sheet: &str, col: &str) -> Result<f64> {
    let n = column_number(col)?;
    let ws = b.sheet(sheet)?;
    let name = column_name(n as i64)?;
    Ok(ws.column_dimension(&name).map(|c| c.width()).filter(|w| *w > 0.0).unwrap_or(9.140625))
}
