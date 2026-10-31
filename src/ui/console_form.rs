use serde_json::{Map, Value, json};
use unicode_width::UnicodeWidthStr;

use super::console::{Console, filter_hint};
use super::*;
use crate::t;

/// Asks for each field in turn; Enter keeps the current value.
pub(crate) fn form(c: &Console, o: &FormOptions) -> Option<Map<String, Value>> {
    if !o.title.is_empty() {
        c.w(&format!("[{}]\n", o.title));
    }
    if !o.description.is_empty() {
        c.w(&format!("{}\n", o.description));
    }
    if !o.error.is_empty() {
        c.w(&format!("! {}\n", o.error));
    }
    let mut values = Map::new();
    let mut tab = String::new();
    for f in &o.fields {
        if f.tab != tab {
            tab = f.tab.clone();
            c.w(&format!("== {tab} ==\n"));
        }
        if !f.error.is_empty() {
            c.w(&format!("! {}: {}\n", field_label(f), f.error));
        }
        if f.kind == "table" && f.select == SELECT_NONE {
            c.w(&format!("{}\n", field_label(f)));
            c.w(&render_table(&f.columns, &f.rows));
        } else {
            let v = ask_field(c, f)?;
            values.insert(f.name.clone(), v);
        }
        if f.kind == "table" && !f.editable.is_empty() {
            let edits = ask_edits(c, f)?;
            if !edits.is_empty() {
                let cells = values.entry("$cells").or_insert_with(|| json!({}));
                cells.as_object_mut().unwrap().insert(f.name.clone(), Value::Array(edits));
            }
        }
    }
    if !o.buttons.is_empty() {
        let b = ask_button(c, o)?;
        values.insert("$button".into(), b);
    }
    Some(values)
}

/// Asks which of the form's buttons to press: the index of a button, or
/// null for the submit button (listed last, the default).
fn ask_button(c: &Console, o: &FormOptions) -> Option<Value> {
    let mut opts: Vec<FormOption> =
        o.buttons.iter().enumerate().map(|(i, b)| FormOption { value: i.to_string(), label: b.clone() }).collect();
    let mut cur = "0";
    if !o.submit.is_empty() {
        opts.push(FormOption { value: String::new(), label: o.submit.clone() });
        cur = "";
    }
    let v = ask_choice(c, &t!("ui.formButton"), &opts, cur)?;
    Some(match v.as_str().and_then(|s| s.parse::<i64>().ok()) {
        Some(n) => json!(n),
        None => Value::Null,
    })
}

fn field_label(f: &FormField) -> String {
    if f.label.is_empty() { f.name.clone() } else { f.label.clone() }
}

fn value_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(value_string).collect::<Vec<_>>().join(";"),
        Value::Number(n) => {
            // Integers print without a fraction, like Go's fmt.
            match n.as_f64() {
                Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
                _ => n.to_string(),
            }
        }
        x => x.to_string(),
    }
}

fn is_yes(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "y" | "yes" | "はい" | "ok" | "1" | "true")
}

fn ask_field(c: &Console, f: &FormField) -> Option<Value> {
    let mut label = field_label(f);
    if f.required {
        label += " *";
    }
    if !f.help.is_empty() {
        c.w(&format!("  {}\n", f.help));
    }
    let cur = value_string(&f.value);
    match f.kind.as_str() {
        "checkbox" => {
            let on = f.value.as_bool().unwrap_or(false);
            let hint = if on { "[Y/n]" } else { "[y/N]" };
            c.w(&format!("{label} {hint}: "));
            let Some(line) = c.read_line() else {
                c.w("\n");
                return None;
            };
            if line.trim().is_empty() {
                return Some(json!(on));
            }
            return Some(json!(is_yes(&line)));
        }
        "select" | "radio" => return ask_choice(c, &label, &f.options, &cur),
        "table" => return ask_rows(c, &label, f),
        "password" => return c.read_password(&label, &cur),
        "files" => {
            label += &filter_hint(&accept_filters(&f.accept));
            label += &t!("ui.multipleHint");
            let line = c.prompt("", &label, &cur)?;
            let out: Vec<Value> = line
                .split(';')
                .map(|p| p.trim().trim_matches('"').to_string())
                .filter(|p| !p.is_empty())
                .map(Value::String)
                .collect();
            return Some(Value::Array(out));
        }
        "file" | "saveFile" => label += &filter_hint(&accept_filters(&f.accept)),
        _ => {}
    }
    let mut line = c.prompt("", &label, &cur)?;
    if matches!(f.kind.as_str(), "file" | "saveFile" | "folder") {
        line = line.trim().trim_matches('"').to_string();
    }
    Some(Value::String(line))
}

/// Shows a table field and asks for the numbers of the rows to select; the
/// value is their 0-based indices.
fn ask_rows(c: &Console, label: &str, f: &FormField) -> Option<Value> {
    c.w(&format!("{label}\n"));
    c.w(&render_table(&f.columns, &f.rows));
    let mut cur = Vec::new();
    let mut nums = Vec::new();
    if let Some(list) = f.value.as_array() {
        for v in list {
            if let Some(n) = v.as_f64() {
                cur.push(json!(n as i64));
                nums.push((n as i64 + 1).to_string());
            }
        }
    }
    let prompt = if nums.is_empty() { t!("ui.tablePick") } else { t!("ui.tableKeep", nums.join(",")) };
    loop {
        let line = c.prompt("", &prompt, "")?;
        match line.trim() {
            "" => return Some(Value::Array(cur)),
            "-" => return Some(json!([])),
            line => {
                let Some(mut sel) = parse_selection(line, f.rows.len()) else {
                    c.w(&format!("! {}\n", t!("ui.badChoice", f.rows.len())));
                    continue;
                };
                if f.select == SELECT_SINGLE {
                    sel.truncate(1);
                }
                return Some(Value::Array(sel.into_iter().map(|n| json!(n)).collect()));
            }
        }
    }
}

/// Shows the errors of an editable table and asks for the rows to change,
/// then for each editable cell of the row; it returns the edits as
/// [row, column, value].
fn ask_edits(c: &Console, f: &FormField) -> Option<Vec<Value>> {
    for e in &f.cell_errors {
        c.w(&format!("! {}\n", cell_error_text(f, e)));
    }
    let mut rows = f.rows.clone();
    let mut edits = Vec::new();
    loop {
        let line = c.prompt("", &t!("ui.tableEdit"), "")?;
        let line = line.trim();
        if line.is_empty() {
            return Some(edits);
        }
        let n: usize = match line.parse() {
            Ok(n) if n >= 1 && n <= rows.len() => n,
            _ => {
                c.w(&format!("! {}\n", t!("ui.badChoice", rows.len())));
                continue;
            }
        };
        let mut row = rows[n - 1].clone();
        for col in &f.editable {
            if col.index < 0 || col.index as usize >= f.columns.len() {
                continue;
            }
            let ci = col.index as usize;
            while row.len() <= ci {
                row.push(String::new());
            }
            let label = &f.columns[ci];
            let v = match col.kind.as_str() {
                "select" => ask_choice(c, label, &col.options, &row[ci])?,
                "checkbox" => json!(is_yes(&c.prompt("", &format!("{label} [y/n]"), &row[ci])?)),
                _ => Value::String(c.prompt("", label, &row[ci])?),
            };
            row[ci] = value_string(&v);
            edits.push(json!([n - 1, col.index, v]));
        }
        rows[n - 1] = row;
    }
}

/// "row 3 金額: message" for an error of a table cell.
pub(crate) fn cell_error_text(f: &FormField, e: &FormCellError) -> String {
    if e.col >= 0 && (e.col as usize) < f.columns.len() {
        t!("form.cellError", e.row + 1, f.columns[e.col as usize], e.message)
    } else {
        t!("form.rowError", e.row + 1, e.message)
    }
}

fn accept_filters(accept: &[String]) -> Vec<FileFilter> {
    if accept.is_empty() {
        return Vec::new();
    }
    vec![FileFilter { description: String::new(), patterns: accept.iter().map(|a| format!("*{a}")).collect() }]
}

/// Lists the options and accepts a number, a value or a label.
fn ask_choice(c: &Console, label: &str, opts: &[FormOption], cur: &str) -> Option<Value> {
    c.w(&format!("{label}\n"));
    let mut def = String::new();
    for (i, o) in opts.iter().enumerate() {
        let mark = if o.value == cur {
            def = (i + 1).to_string();
            "*"
        } else {
            " "
        };
        c.w(&format!(" {mark}{}) {}\n", i + 1, o.label));
    }
    loop {
        let line = c.prompt("", &t!("ui.choose"), &def)?;
        let line = line.trim();
        if line.is_empty() {
            return Some(Value::String(cur.to_string()));
        }
        if let Ok(n) = line.parse::<usize>()
            && n >= 1
            && n <= opts.len()
        {
            return Some(Value::String(opts[n - 1].value.clone()));
        }
        if let Some(o) = opts.iter().find(|o| line == o.value || line == o.label) {
            return Some(Value::String(o.value.clone()));
        }
        c.w(&format!("! {}\n", t!("ui.badChoice", opts.len())));
    }
}

/// Prints the rows with 1-based numbers and, when selecting, asks for the
/// numbers of the rows ("1,3-5").
pub(crate) fn table(c: &Console, o: &TableOptions) -> Option<Vec<i64>> {
    if !o.title.is_empty() {
        c.w(&format!("[{}]\n", o.title));
    }
    c.w(&render_table(&o.columns, &o.rows));
    if o.select == SELECT_NONE {
        return Some(Vec::new());
    }
    loop {
        let line = c.prompt("", &t!("ui.tableSelect"), "")?;
        if line.trim().is_empty() {
            return None;
        }
        let Some(mut sel) = parse_selection(&line, o.rows.len()) else {
            c.w(&format!("! {}\n", t!("ui.badChoice", o.rows.len())));
            continue;
        };
        if o.select == SELECT_SINGLE {
            sel.truncate(1);
        }
        return Some(sel);
    }
}

/// Parses "1, 3-5" into 0-based indices below n.
pub(crate) fn parse_selection(s: &str, n: usize) -> Option<Vec<i64>> {
    let mut out = Vec::new();
    for part in s.split([',', ' ', '、']).filter(|p| !p.is_empty()) {
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (a, Some(b)),
            None => (part, None),
        };
        let a: usize = lo.parse().ok().filter(|a| *a >= 1 && *a <= n)?;
        let b = match hi {
            Some(h) => h.parse().ok().filter(|b| *b >= a && *b <= n)?,
            None => a,
        };
        for i in a..=b {
            let v = (i - 1) as i64;
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Draws a box table like console.table, with a number column.
pub(crate) fn render_table(cols: &[String], rows: &[Vec<String>]) -> String {
    let mut header = vec!["#".to_string()];
    header.extend(cols.iter().cloned());
    let clean = |s: &str| s.replace("\r\n", " ").replace(['\n', '\t'], " ");
    let mut widths: Vec<usize> = header.iter().map(|h| h.width()).collect();
    let mut body = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut line = vec![String::new(); header.len()];
        line[0] = (r + 1).to_string();
        for i in 0..cols.len() {
            if let Some(cell) = row.get(i) {
                line[i + 1] = clean(cell);
            }
        }
        for (i, cell) in line.iter().enumerate() {
            widths[i] = widths[i].max(cell.width());
        }
        body.push(line);
    }
    let mut b = String::new();
    let rule = |b: &mut String, l: &str, m: &str, r: &str| {
        b.push_str(l);
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                b.push_str(m);
            }
            b.push_str(&"─".repeat(w + 2));
        }
        b.push_str(r);
        b.push('\n');
    };
    let line = |b: &mut String, cells: &[String]| {
        b.push('│');
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                b.push('│');
            }
            b.push(' ');
            b.push_str(cell);
            b.push_str(&" ".repeat(widths[i] - cell.width()));
            b.push(' ');
        }
        b.push_str("│\n");
    };
    rule(&mut b, "┌", "┬", "┐");
    line(&mut b, &header);
    rule(&mut b, "├", "┼", "┤");
    for r in &body {
        line(&mut b, r);
    }
    rule(&mut b, "└", "┴", "┘");
    b
}
