use std::path::Path;
use std::sync::Arc;

use super::ConsoleState;
use crate::ui::{FileFilter, PickerOptions, Ui};
use crate::{Error, Result, err, t, vba};

/// The workbooks offered by the picker for extracting VBA.
pub(crate) const MACRO_PATTERNS: &[&str] = &["*.xlsm", "*.xlsb", "*.xlam", "*.xltm", "*.xls"];

/// Whether path is an Excel workbook (with or without macros) rather than a
/// script.
pub(crate) fn is_workbook(path: &str) -> bool {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "xlsm" | "xlsb" | "xlam" | "xltm" | "xls" | "xla" | "xlt" | "xlsx" | "xltx")
}

fn base(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The VBA code of the workbooks as one text and the number of modules.
/// Every workbook must have macros.
pub(crate) fn vba_text(books: &[String]) -> Result<(String, usize)> {
    let mut parts = Vec::new();
    let mut modules = 0;
    for b in books {
        let Some(p) = vba::read_file(b)? else {
            return Err(err!("cli.vbaNone", base(b)));
        };
        parts.push(vba::text(&base(b), &p));
        modules += p.modules.len();
    }
    Ok((parts.join("\n"), modules))
}

/// The default text file for the workbook: 売上.xlsm → 売上_VBA.txt.
pub(crate) fn vba_output(book: &str) -> String {
    let p = Path::new(book);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    p.with_file_name(format!("{stem}_VBA.txt")).to_string_lossy().into_owned()
}

/// xljs vba <book>... [-o out.txt]: the VBA code of the workbooks in one
/// text file (next to the first workbook by default, - for stdout).
pub(crate) fn cmd_vba(con: &ConsoleState, args: &[String]) -> i32 {
    let (out, books) = match super::split_out(args) {
        Ok(x) => x,
        Err(e) => return con.fail(None, &e),
    };
    if books.is_empty() {
        eprintln!("{}", t!("cli.usageVBA"));
        return 2;
    }
    let (text, _) = match vba_text(&books) {
        Ok(x) => x,
        Err(e) => return con.fail(None, &e),
    };
    if out.as_deref() == Some("-") {
        print!("{text}");
        return 0;
    }
    let out = out.unwrap_or_else(|| vba_output(&books[0]));
    if let Err(e) = std::fs::write(&out, text) {
        return con.fail(None, &Error::path("open", out, e));
    }
    println!("{}", t!("cli.created", out));
    0
}

/// Handles workbooks given instead of a script (dropped onto xljs.exe or
/// chosen in its picker): asks where to save their VBA code as one text
/// file and offers to open it. Without the native UI the text is written
/// next to the first workbook.
pub(crate) fn drop_vba(con: &ConsoleState, u: Arc<dyn Ui>, books: &[String]) -> i32 {
    let (text, modules) = match vba_text(books) {
        Ok(x) => x,
        Err(e) => return con.fail(Some(&*u), &e),
    };
    let mut out = vba_output(&books[0]);
    let native = u.name() == "native";
    if native {
        let abs = std::path::absolute(&out).unwrap_or_else(|_| out.clone().into());
        let p = u.save_file(&PickerOptions {
            title: t!("cli.vbaSaveTitle"),
            start_in: abs.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            suggested_name: abs.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            filters: vec![FileFilter { description: t!("cli.vbaText"), patterns: vec!["*.txt".into()] }],
            ..Default::default()
        });
        let Some(p) = p else { return 0 };
        out = p;
    }
    if let Err(e) = std::fs::write(&out, text) {
        return con.fail(Some(&*u), &Error::path("open", out, e));
    }
    if !native {
        println!("{}", t!("cli.created", out));
        return 0;
    }
    if u.confirm("xljs", &t!("cli.vbaSaved", modules, out))
        && let Err(e) = u.open_path(&out)
    {
        return con.fail(Some(&*u), &e);
    }
    0
}
