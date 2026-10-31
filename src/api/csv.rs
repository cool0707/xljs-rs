use rquickjs::Value;

use super::N;
use super::js::{self, elems, nullish};

/// Parses text into records like Go's encoding/csv with LazyQuotes and
/// FieldsPerRecord = -1: rows may have different lengths, quotes inside
/// unquoted fields are kept, line breaks inside quoted fields are kept
/// (CRLF becomes LF). Empty lines and lines starting with comment are
/// skipped.
pub(crate) fn parse_csv(text: &str, delim: char, comment: Option<char>) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let n = chars.len();
    while i < n {
        // Start of a record: skip empty lines and comments.
        if chars[i] == '\n' || (chars[i] == '\r' && i + 1 < n && chars[i + 1] == '\n') {
            i += if chars[i] == '\r' { 2 } else { 1 };
            continue;
        }
        if Some(chars[i]) == comment {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            i += 1;
            continue;
        }
        let mut rec = Vec::new();
        loop {
            let mut field = String::new();
            if i < n && chars[i] == '"' {
                // Quoted field.
                i += 1;
                loop {
                    if i >= n {
                        break;
                    }
                    let c = chars[i];
                    if c == '"' {
                        if i + 1 < n && chars[i + 1] == '"' {
                            field.push('"');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        // Lazy quotes: text after the closing quote up to
                        // the delimiter belongs to the field.
                        if i < n && chars[i] != delim && chars[i] != '\n' && !(chars[i] == '\r' && i + 1 < n && chars[i + 1] == '\n') && chars[i] != '\r' {
                            field.push('"');
                            continue;
                        }
                        break;
                    }
                    if c == '\r' && i + 1 < n && chars[i + 1] == '\n' {
                        field.push('\n');
                        i += 2;
                        continue;
                    }
                    field.push(c);
                    i += 1;
                }
            } else {
                while i < n && chars[i] != delim && chars[i] != '\n' && !(chars[i] == '\r' && (i + 1 == n || chars[i + 1] == '\n')) {
                    field.push(chars[i]);
                    i += 1;
                }
            }
            rec.push(field);
            if i < n && chars[i] == delim {
                i += 1;
                continue;
            }
            // End of record.
            if i < n && chars[i] == '\r' {
                i += 1;
            }
            if i < n && chars[i] == '\n' {
                i += 1;
            }
            break;
        }
        out.push(rec);
    }
    out
}

fn needs_quotes(f: &str, delim: char) -> bool {
    if f.is_empty() {
        return false;
    }
    if f.contains(delim) || f.contains(['"', '\r', '\n']) {
        return true;
    }
    f.starts_with([' ', '\t'])
}

/// Writes records. Line breaks inside fields are kept as-is (Excel stores
/// in-cell line breaks as LF), only record separators use eol.
pub(crate) fn stringify_csv(rows: &[Vec<String>], delim: char, crlf: bool, quote_all: bool) -> String {
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut b = String::new();
    for row in rows {
        for (i, f) in row.iter().enumerate() {
            if i > 0 {
                b.push(delim);
            }
            if quote_all || needs_quotes(f, delim) {
                b.push('"');
                b.push_str(&f.replace('"', "\"\""));
                b.push('"');
            } else {
                b.push_str(f);
            }
        }
        b.push_str(eol);
    }
    b
}

fn delim_char(s: &str, def: char) -> char {
    s.chars().next().unwrap_or(def)
}

pub(super) fn install(n: &N) {
    // csvParse(text, delimiter, comment) -> string[][]
    n.set("csvParse", |_, a| {
        let text = a.str(0);
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        let comment = a.str(2).chars().next();
        let rows = parse_csv(text, delim_char(&a.str(1), ','), comment);
        let vals: Vec<Value> = rows.into_iter().map(|r| js::v(&a.ctx, r)).collect();
        js::array(&a.ctx, vals)
    });
    // csvStringify(rows: string[][], delimiter, crlf, quoteAll) -> string
    n.set("csvStringify", |_, a| {
        let rows: Vec<Vec<String>> = elems(&a.arg(0))
            .iter()
            .map(|r| elems(r).iter().map(|e| if nullish(e) { String::new() } else { js::to_str(e) }).collect())
            .collect();
        a.ret(stringify_csv(&rows, delim_char(&a.str(1), ','), a.bool(2), a.bool(3)))
    });
}
