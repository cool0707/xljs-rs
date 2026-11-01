//! The new script wizard of the development mode: the user's answers
//! (port an Excel macro or start afresh, form or not, app or not, the
//! name, what it should do, the input and output files) become the first
//! message for the AI chat. For a port, the macros (VBA) and an outline of
//! the workbook (sheets, first rows, formulas) go with it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Result, t};

/// The answers of the wizard (dev.js).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Answers {
    /// "new" or "port" (an Excel macro).
    pub kind: String,
    /// The workbooks of a port.
    pub books: Vec<String>,
    pub include_vba: bool,
    pub include_sheets: bool,
    /// The first rows of each sheet put in the outline (0 = none).
    pub sample_rows: u32,
    /// "yes", "no" or "auto" (up to the AI), with a description.
    pub input: String,
    pub input_note: String,
    pub output: String,
    pub output_note: String,
    /// "yes" (ui.form), "no" (no window at all) or "auto".
    pub form: String,
    /// "script", "app" or "auto".
    pub mode: String,
    /// "name" (name), "auto" (the AI chooses) or "later" (a temporary name).
    pub name_mode: String,
    pub name: String,
    /// What the script should do.
    pub overview: String,
    /// Puts the instructions for AIs (docs/ai/prompt.md) first.
    pub intro: bool,
}

/// What a workbook holds, for the wizard's list.
#[derive(Debug, Clone, Serialize)]
pub struct BookInfo {
    pub path: String,
    pub name: String,
    /// The number of VBA modules (0 = no macros).
    pub modules: usize,
    /// Why it could not be read ("" = fine).
    pub error: String,
}

fn base(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The script file name for the answers: the name given (with .js), else
/// a temporary name.
pub fn file_name(a: &Answers) -> String {
    let name = a.name.trim();
    let name: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    if a.name_mode == "name" && !name.is_empty() {
        let lower = name.to_lowercase();
        if [".js", ".ts", ".mjs"].iter().any(|e| lower.ends_with(e)) {
            return name;
        }
        return format!("{name}.js");
    }
    format!("{}.js", t!("dev.newScriptName"))
}

/// A path for name in dir that is not taken yet.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut path = dir.join(name);
    let mut i = 2;
    while path.exists() {
        path = dir.join(format!("{stem}{i}{ext}"));
        i += 1;
    }
    path
}

/// Reads what the workbooks hold.
pub fn books(paths: &[String]) -> Vec<BookInfo> {
    paths
        .iter()
        .map(|p| {
            let (modules, error) = match crate::vba::read_file(p) {
                Ok(Some(proj)) => (proj.modules.len(), String::new()),
                Ok(None) => (0, String::new()),
                Err(e) => (0, crate::i18n::error(&e)),
            };
            BookInfo { path: p.clone(), name: base(p), modules, error }
        })
        .collect()
}

/// The macros and the outlines of the workbooks of a port, as the
/// answers ask (Markdown).
pub fn workbook_text(a: &Answers) -> String {
    let mut s = String::new();
    if a.include_vba {
        for b in &a.books {
            if let Ok(Some(p)) = crate::vba::read_file(b) {
                s.push_str(&crate::vba::text(&base(b), &p));
                s.push('\n');
            }
        }
    }
    if a.include_sheets && !a.books.is_empty() {
        s.push_str(&format!("## {}\n\n", t!("wizard.sheetsTitle")));
        for b in &a.books {
            match crate::api::workbook_outline(b, a.sample_rows as usize) {
                Ok(o) => s.push_str(&o),
                Err(e) => s.push_str(&format!("### {}\n\n{}\n", base(b), crate::i18n::error(&e))),
            }
            s.push('\n');
        }
    }
    s
}

fn choice(v: &str, yes: &str, no: &str, auto: &str) -> String {
    match v {
        "yes" | "name" | "script" => t!(yes),
        "no" | "app" | "later" => t!(no),
        _ => t!(auto),
    }
}

/// The first message for the AI chat.
pub fn prompt(a: &Answers) -> String {
    let port = a.kind == "port";
    let file = file_name(a);
    let mut s = String::new();
    if a.intro {
        s.push_str(super::AI_PROMPT.trim_end());
        s.push_str("\n\n---\n\n");
    }
    s.push_str(&format!("# {}\n\n", t!(if port { "wizard.titlePort" } else { "wizard.titleNew" })));
    s.push_str(&format!("{}\n\n", t!(if port { "wizard.leadPort" } else { "wizard.leadNew" })));

    // The conditions.
    let mut items = Vec::new();
    let mode = choice(&a.mode, "wizard.modeScript", "wizard.modeApp", "wizard.modeAuto");
    items.push(format!("{}: {mode}", t!("wizard.modeLabel")));
    if a.mode != "app" {
        items.push(format!("{}: {}", t!("wizard.formLabel"), choice(&a.form, "wizard.formYes", "wizard.formNo", "wizard.formAuto")));
    }
    let name = match a.name_mode.as_str() {
        "name" if !a.name.trim().is_empty() => t!("wizard.nameGiven", file),
        "later" => t!("wizard.nameLater", file),
        _ => t!("wizard.nameAuto"),
    };
    items.push(format!("{}: {name}", t!("wizard.nameLabel")));
    if port {
        let books: Vec<String> = a.books.iter().map(|b| base(b)).collect();
        if !books.is_empty() {
            items.push(format!("{}: {}", t!("wizard.booksLabel"), books.join(", ")));
        }
    }
    for (label, v, note, yes, no, auto) in [
        ("wizard.inputLabel", &a.input, &a.input_note, "wizard.inputYes", "wizard.inputNo", "wizard.inputAuto"),
        ("wizard.outputLabel", &a.output, &a.output_note, "wizard.outputYes", "wizard.outputNo", "wizard.outputAuto"),
    ] {
        if !port && v.is_empty() && note.trim().is_empty() {
            continue;
        }
        let mut line = format!("{}: {}", t!(label), choice(v, yes, no, auto));
        if !note.trim().is_empty() {
            line.push_str(&format!(" — {}", note.trim().replace('\n', " / ")));
        }
        items.push(line);
    }
    s.push_str(&format!("## {}\n\n", t!("wizard.conditions")));
    for i in items {
        s.push_str(&format!("- {i}\n"));
    }
    s.push('\n');

    // What it should do.
    let overview = a.overview.trim();
    if !overview.is_empty() {
        s.push_str(&format!("## {}\n\n{overview}\n\n", t!(if port { "wizard.overviewPort" } else { "wizard.overviewNew" })));
    } else if port {
        s.push_str(&format!("## {}\n\n{}\n\n", t!("wizard.overviewPort"), t!("wizard.overviewSame")));
    }

    if port {
        let w = workbook_text(a);
        if !w.trim().is_empty() {
            s.push_str(&w);
            s.push('\n');
        }
    }

    // How to answer.
    s.push_str(&format!("## {}\n\n", t!("wizard.askTitle")));
    if port {
        s.push_str(&format!("- {}\n", t!("wizard.askExplain")));
    }
    s.push_str(&format!("- {}\n", t!("wizard.askAssume")));
    let out = if a.name_mode == "auto" || (a.name_mode == "name" && a.name.trim().is_empty()) { t!("wizard.askFileAuto") } else { t!("wizard.askFile", file) };
    s.push_str(&format!("- {out}\n"));
    if a.mode == "app" {
        s.push_str(&format!("- {}\n", t!("wizard.askApp")));
    }
    s
}

/// The file the wizard creates before the AI answers: a script that only
/// says so, or for an app the app's frame.
pub fn placeholder(title: &str) -> String {
    format!("// ==xljs==\n// @title {title}\n// ==/xljs==\n\n// {}\n", t!("wizard.placeholder"))
}

/// Whether a workbook can be given to the wizard.
pub fn is_book(p: &str) -> bool {
    let e = super::project::ext_of(p);
    matches!(e.as_str(), "xlsm" | "xlsb" | "xlam" | "xltm" | "xls" | "xlsx" | "xltx")
}

/// Writes the macros and outlines next to the first workbook (for chats
/// that take attachments rather than long messages).
pub fn save_text(a: &Answers, path: &Path) -> Result<()> {
    super::project::write_text(path, &workbook_text(a))
}
