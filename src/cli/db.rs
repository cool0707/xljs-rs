//! SQLite databases given to xljs instead of a script (dropped onto
//! xljs.exe) and `xljs repair`: check the file and repair it when it is
//! damaged (see crate::db::repair).

use std::path::Path;
use std::sync::Arc;

use super::ConsoleState;
use crate::db::{self, Options, Report};
use crate::ui::Ui;
use crate::{Result, t};

/// Whether path is a SQLite database rather than a script.
pub(crate) fn is_database(path: &str) -> bool {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "db" | "sqlite" | "sqlite3" | "db3")
}

fn backups_note(path: &Path) -> String {
    let snaps = db::share::Sidecar::find(path).map(|s| s.snapshots()).unwrap_or_default();
    match snaps.last() {
        Some(s) => t!("cli.dbBackups", snaps.len(), s.time.format("%Y-%m-%d %H:%M")),
        None => String::new(),
    }
}

/// The message for a repair, like the one scripts show.
fn report_text(path: &str, r: &Report) -> String {
    let backup = r.backup.clone().map(|b| b.chars().take(16).collect::<String>()).unwrap_or_default();
    let mut lines = vec![t!(&format!("sqlite.repaired.{}", r.method), path, backup, r.replayed, r.skipped)];
    if !r.notes.is_empty() {
        lines.push(String::new());
        lines.extend(r.notes.iter().cloned());
    }
    if let Some(b) = &r.broken {
        lines.push(String::new());
        lines.push(t!("sqlite.brokenKept", b));
    }
    lines.join("\n")
}

/// Checks path; when damaged, asks (unless yes) and repairs it.
fn check_and_repair(path: &str, force: bool, ask: &dyn Fn(&str) -> bool) -> Result<Option<String>> {
    let abs = std::path::absolute(path).map_err(|e| crate::Error::path("open", path, e))?;
    let problems = if abs.exists() { db::check_file(&abs).unwrap_or_else(|e| vec![crate::i18n::error(&e)]) } else { vec![t!("sqlite.missing")] };
    let ok = problems.len() == 1 && problems[0] == "ok";
    if ok && !force {
        return Ok(Some(t!("cli.dbOk", path, backups_note(&abs))));
    }
    if !ok && !ask(&t!("cli.dbAskRepair", path, problems.join("\n"))) {
        return Ok(None);
    }
    let r = db::repair(&abs, force, &Options::default())?;
    Ok(Some(report_text(path, &r)))
}

/// xljs repair <file.db> [--force]
pub(crate) fn cmd_repair(con: &ConsoleState, args: &[String]) -> i32 {
    let force = args.iter().any(|a| a == "--force" || a == "-force");
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if files.len() != 1 {
        eprintln!("{}", t!("cli.usageRepair"));
        return 2;
    }
    match check_and_repair(files[0], force, &|_| true) {
        Ok(Some(msg)) => {
            println!("{msg}");
            0
        }
        Ok(None) => 1,
        Err(e) => con.fail(None, &e),
    }
}

/// A database dropped onto xljs.exe: check it, offer to repair it.
pub(crate) fn drop_database(con: &ConsoleState, u: Arc<dyn Ui>, files: &[String]) -> i32 {
    for f in files {
        match check_and_repair(f, false, &|m| u.confirm("xljs", m)) {
            Ok(Some(msg)) => u.alert("xljs", &msg),
            Ok(None) => {}
            Err(e) => return con.fail(Some(&*u), &e),
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_from_the_command() {
        let _g = crate::testutil::lock_lang("en");
        let dir = crate::testutil::tempdir();
        let p = dir.join("a.db");
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES(1);").unwrap();
        drop(c);
        assert!(is_database("x.DB") && is_database("y.sqlite") && !is_database("y.js"));
        let ps = p.to_string_lossy().into_owned();
        let msg = check_and_repair(&ps, false, &|_| panic!("not damaged")).unwrap().unwrap();
        assert!(msg.contains("not damaged"), "{msg}");
        std::fs::write(&p, b"this is not a database at all, really not one").unwrap();
        assert!(check_and_repair(&ps, false, &|_| false).unwrap().is_none());
        // Nothing left to salvage from: the repair fails and the file stays.
        assert!(check_and_repair(&ps, false, &|_| true).is_err());
        assert!(p.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
