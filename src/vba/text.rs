use crate::t;

use super::{CLASS, FORM, Module, Project, STANDARD};

/// Formats the project of the workbook named file as Markdown for pasting
/// into an AI chat: a heading per module and its code in a vba code block.
/// The Attribute lines at the top of the modules are left out, and so are
/// modules without code (only Option statements), which are listed. Without
/// a file name the project's name is used.
pub fn text(file: &str, p: &Project) -> String {
    let file = if file.is_empty() { p.name.as_str() } else { file };
    let mut b = String::new();
    b.push_str(&t!("vba.textTitle", file));
    b.push_str("\n\n");
    b.push_str(&t!("vba.textIntro", p.name, file, p.modules.len()));
    b.push('\n');
    let mut empty = Vec::new();
    for m in &p.modules {
        let code = body(&m.code);
        if code.is_empty() {
            empty.push(m.name.clone());
            continue;
        }
        let fence = "`".repeat(3.max(longest_run(&code, '`') + 1));
        b.push('\n');
        b.push_str(&t!("vba.textModule", m.name, describe(m)));
        b.push_str("\n\n");
        b.push_str(&format!("{fence}vba\n{code}\n{fence}\n"));
    }
    if !empty.is_empty() {
        b.push('\n');
        b.push_str(&t!("vba.textEmpty", empty.join(&t!("vba.separator"))));
        b.push('\n');
    }
    b
}

fn describe(m: &Module) -> String {
    match m.kind.as_str() {
        STANDARD => return t!("vba.type.standard"),
        CLASS => return t!("vba.type.class"),
        FORM => return t!("vba.type.form"),
        _ => {}
    }
    if !m.sheet.is_empty() {
        t!("vba.type.sheet", m.sheet)
    } else if m.name.eq_ignore_ascii_case("ThisWorkbook") {
        t!("vba.type.workbook")
    } else {
        t!("vba.type.document")
    }
}

/// The code without the leading Attribute lines and the blank lines around
/// it, or "" when nothing but Option statements remain.
fn body(code: &str) -> String {
    let lines: Vec<&str> = code.split('\n').skip_while(|l| l.starts_with("Attribute VB_")).collect();
    let s = lines.join("\n").trim_matches([' ', '\t', '\n']).to_string();
    for l in s.split('\n') {
        let l = l.trim();
        if !l.is_empty() && !l.to_lowercase().starts_with("option ") {
            return s;
        }
    }
    String::new()
}

fn longest_run(s: &str, c: char) -> usize {
    let (mut best, mut n) = (0, 0);
    for ch in s.chars() {
        if ch == c {
            n += 1;
            best = best.max(n);
        } else {
            n = 0;
        }
    }
    best
}
