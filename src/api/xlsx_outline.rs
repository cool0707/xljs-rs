//! An outline of a workbook for an AI chat (the development mode's new
//! script wizard): its sheets, their size, the first rows and the
//! formulas, grouped when they have the same shape (=B2*C2, =B3*C3, ...).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::xlsx::{Book, cell_text, column_name, open_bytes, rows};
use crate::{Error, Result, t};

/// Formulas listed per sheet (groups of the same shape count as one).
const MAX_FORMULAS: usize = 40;
/// Columns shown of the first rows.
const MAX_COLUMNS: usize = 15;
/// Characters of a cell shown.
const MAX_CELL: usize = 40;

static REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\$?)\b([A-Z]{1,3})(\$?)([0-9]+)\b").unwrap());

/// The formula with its references made relative to (col, row), so that
/// formulas copied down or across look the same.
fn shape(formula: &str, col: u32, row: u32) -> String {
    REF.replace_all(formula, |c: &regex::Captures| {
        let cn = super::xlsx::column_number(&c[2]).unwrap_or(0) as i64;
        let rn: i64 = c[4].parse().unwrap_or(0);
        let cpart = if c[1].is_empty() { format!("C[{}]", cn - col as i64) } else { format!("C{cn}") };
        let rpart = if c[3].is_empty() { format!("R[{}]", rn - row as i64) } else { format!("R{rn}") };
        format!("{rpart}{cpart}")
    })
    .into_owned()
}

fn addr(col: u32, row: u32) -> String {
    format!("{}{row}", column_name(col as i64).unwrap_or_default())
}

fn cut(s: &str) -> String {
    let s = s.replace(['\r', '\n'], " ").replace('|', "\\|");
    let c: Vec<char> = s.chars().collect();
    if c.len() > MAX_CELL { format!("{}…", c[..MAX_CELL].iter().collect::<String>()) } else { s }
}

/// The outline of the workbook at path (Markdown) with its first
/// sample_rows rows.
pub fn outline(path: &str, sample_rows: usize) -> Result<String> {
    let data = std::fs::read(path).map_err(|e| Error::path("open", path, e))?;
    let name = std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        // .xls (or an encrypted workbook): only its macros can be read.
        return Ok(format!("### {name}\n\n{}\n", t!("wizard.oldFormat")));
    }
    let b = open_bytes(&data, path.to_string())?;
    let mut out = format!("### {name}\n");
    for ws in b.wb.sheet_collection() {
        out.push('\n');
        out.push_str(&sheet(&b, ws, sample_rows));
    }
    Ok(out)
}

fn sheet(b: &Book, ws: &umya_spreadsheet::Worksheet, sample_rows: usize) -> String {
    let all = rows(ws);
    // The size counts formulas without a computed value too.
    let mut last_col = all.iter().filter_map(|r| r.last().map(|(c, _)| *c)).max().unwrap_or(0);
    let mut last_row = all.len() as u32;
    for c in ws.cells() {
        if c.formula_obj().is_some() {
            last_col = last_col.max(c.coordinate().col_num());
            last_row = last_row.max(c.coordinate().row_num());
        }
    }
    let mut s = if last_row == 0 {
        format!("{}\n", t!("wizard.sheetEmpty", ws.name()))
    } else {
        format!("{}\n", t!("wizard.sheetHead", ws.name(), addr(last_col, last_row), last_row, last_col))
    };
    // The first rows as a table.
    let shown: Vec<_> = all.iter().take(sample_rows).collect();
    if !shown.is_empty() {
        let cols = (last_col as usize).min(MAX_COLUMNS);
        s.push_str(&format!("\n{}\n\n", t!("wizard.firstRows", shown.len())));
        let head: Vec<String> = (1..=cols).map(|c| column_name(c as i64).unwrap_or_default()).collect();
        s.push_str(&format!("| | {} |\n|---|{}\n", head.join(" | "), "---|".repeat(cols)));
        for (i, r) in shown.iter().enumerate() {
            let mut cells = vec![String::new(); cols];
            for (c, cell) in r.iter() {
                if (*c as usize) <= cols {
                    cells[*c as usize - 1] = cut(&cell_text(b, cell));
                }
            }
            s.push_str(&format!("| {} | {} |\n", i + 1, cells.join(" | ")));
        }
        if last_col as usize > cols {
            s.push_str(&format!("{}\n", t!("wizard.moreColumns", last_col as usize - cols)));
        }
    }
    // The formulas, by shape.
    struct Group {
        first: (u32, u32),
        last: (u32, u32),
        formula: String,
        count: usize,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut shared: HashMap<u32, String> = HashMap::new();
    let mut cells: Vec<&umya_spreadsheet::Cell> = ws.cells().into_iter().filter(|c| c.formula_obj().is_some()).collect();
    cells.sort_by_key(|c| (c.coordinate().row_num(), c.coordinate().col_num()));
    for c in cells {
        let (col, row) = (c.coordinate().col_num(), c.coordinate().row_num());
        let text = c.formula().to_string();
        let si = c.formula_shared_index();
        let key = if text.is_empty() {
            match si.and_then(|i| shared.get(&i)) {
                Some(k) => k.clone(),
                None => continue,
            }
        } else {
            let k = shape(&text, col, row);
            if let Some(i) = si {
                shared.insert(i, k.clone());
            }
            k
        };
        match by_key.get(&key) {
            Some(&i) => {
                let g = &mut groups[i];
                g.count += 1;
                g.last = (col, row);
            }
            None => {
                by_key.insert(key, groups.len());
                groups.push(Group { first: (col, row), last: (col, row), formula: text, count: 1 });
            }
        }
    }
    if !groups.is_empty() {
        let total: usize = groups.iter().map(|g| g.count).sum();
        s.push_str(&format!("\n{}\n\n", t!("wizard.formulas", total)));
        for g in groups.iter().take(MAX_FORMULAS) {
            let at = addr(g.first.0, g.first.1);
            if g.count > 1 {
                s.push_str(&format!("- {at}: `={}` {}\n", g.formula, t!("wizard.sameShape", at, addr(g.last.0, g.last.1), g.count)));
            } else {
                s.push_str(&format!("- {at}: `={}`\n", g.formula));
            }
        }
        if groups.len() > MAX_FORMULAS {
            s.push_str(&format!("{}\n", t!("wizard.moreFormulas", groups.len() - MAX_FORMULAS)));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        assert_eq!(shape("B2*C2", 4, 2), shape("B3*C3", 4, 3));
        assert_ne!(shape("B2*C2", 4, 2), shape("B2*C3", 4, 3));
        assert_eq!(shape("SUM($B$2:B5)", 2, 5), "SUM(R2C2:R[0]C[0])");
    }
}
