//! Checking and repairing a damaged database.
//!
//! repair() puts the damaged file (with its journal) aside and builds a new
//! one in its place, trying in turn:
//!
//! 1. **journal / reindex / vacuum** on a copy of the damaged file: rolls
//!    back a journal left by a client that crashed, rebuilds the indexes,
//!    rebuilds the whole file. Nothing is lost when one of them makes the
//!    copy pass integrity_check.
//! 2. **backup**: the newest backup that passes the check (shared
//!    databases), with the transactions of the change logs from it on
//!    replayed. Only what was not logged is lost (a client that crashed
//!    between committing and logging, values of non-deterministic SQL
//!    such as datetime('now') come out different).
//! 3. **salvage**: a new database with the schema of the damaged one and
//!    every row that can still be read from it (rows on damaged pages are
//!    lost).

use std::path::{Path, PathBuf};

use chrono::Local;
use rusqlite::Connection;

use super::share::{LogEntry, Sidecar, json_to_value};
use super::{Options, connect, integrity_check, is_ok, quick_check, sql_error};
use crate::{Error, Result, err, t};

/// What repair() did.
#[derive(Debug, Default, serde::Serialize)]
pub struct Report {
    /// "ok" (not damaged, nothing done) or "repaired".
    pub status: String,
    /// "none", "journal", "reindex", "vacuum", "backup" or "salvage".
    pub method: String,
    /// What the check found before the repair.
    pub problems: Vec<String>,
    /// The time of the backup the database was rebuilt from.
    pub backup: Option<String>,
    /// Transactions of the change logs replayed / that failed.
    pub replayed: usize,
    pub skipped: usize,
    /// Rows per table after the repair.
    pub tables: Vec<(String, i64)>,
    /// What could not be carried over (salvage).
    pub notes: Vec<String>,
    /// Where the damaged file was put.
    pub broken: Option<String>,
}

fn open_rw(path: &Path, opts: &Options) -> Result<Connection> {
    connect(path, false, false, false, opts.timeout)
}

/// PRAGMA integrity_check of the database at path: ["ok"], the problems,
/// or the error that kept it from being read.
pub fn check_file(path: &Path) -> Result<Vec<String>> {
    let opts = Options::default();
    let conn = open_rw(path, &opts)?;
    let r = integrity_check(&conn).map_err(sql_error)?;
    Ok(r)
}

fn problems_of(path: &Path) -> Vec<String> {
    if !path.exists() {
        return vec![t!("sqlite.missing")];
    }
    match check_file(path) {
        Ok(v) => v,
        Err(e) => vec![crate::i18n::error(&e)],
    }
}

const SIDE_FILES: &[&str] = &["", "-journal", "-wal", "-shm"];

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn rename(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to).map_err(|e| Error::Link {
        op: "rename".into(),
        old: from.to_string_lossy().into_owned(),
        new: to.to_string_lossy().into_owned(),
        source: e,
    })
}

/// Moves the database and its journal files to dest (dest + "-journal" ...).
fn move_files(from: &Path, to: &Path) -> Result<()> {
    for s in SIDE_FILES {
        let f = with_suffix(from, s);
        if f.exists() {
            rename(&f, &with_suffix(to, s))?;
        }
    }
    Ok(())
}

fn copy_files(from: &Path, to: &Path) -> Result<()> {
    for s in SIDE_FILES {
        let (f, t) = (with_suffix(from, s), with_suffix(to, s));
        let _ = std::fs::remove_file(&t);
        if f.exists() {
            std::fs::copy(&f, &t).map_err(|e| Error::path("copy", f.to_string_lossy(), e))?;
        }
    }
    Ok(())
}

fn remove_files(p: &Path) {
    for s in SIDE_FILES {
        let _ = std::fs::remove_file(with_suffix(p, s));
    }
}

/// Repairs the database at path (see the module documentation). Unless
/// force is set, a database that passes integrity_check is left alone.
pub fn repair(path: &Path, force: bool, opts: &Options) -> Result<Report> {
    let side = Sidecar::find(path);
    let _lock = match &side {
        Some(s) => Some(s.lock(path, true, opts.timeout)?),
        None => None,
    };
    let problems = problems_of(path);
    let mut rep = Report { problems: problems.clone(), ..Default::default() };
    if is_ok(&problems) && !force {
        rep.status = "ok".into();
        rep.method = "none".into();
        return Ok(rep);
    }
    let exists = path.exists();
    let has_backups = side.as_ref().is_some_and(|s| !s.snapshots().is_empty());
    if !exists && !has_backups {
        return Err(Error::path("open", path.to_string_lossy(), std::io::Error::from(std::io::ErrorKind::NotFound)));
    }

    // Keep the damaged file.
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let broken = if exists {
        let dir = match &side {
            Some(s) => {
                let d = s.broken_dir();
                std::fs::create_dir_all(&d).map_err(|e| Error::path("mkdir", d.to_string_lossy(), e))?;
                d
            }
            None => path.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        let dest = dir.join(format!("{stem}.broken-{stamp}{ext}"));
        move_files(path, &dest)?;
        rep.broken = Some(dest.to_string_lossy().into_owned());
        Some(dest)
    } else {
        None
    };
    let tmp = with_suffix(path, ".repairing");
    remove_files(&tmp);

    let mut done = false;
    if let Some(b) = &broken {
        copy_files(b, &tmp)?;
        if let Some(m) = fix_copy(&tmp, opts) {
            rep.method = m.into();
            done = true;
        } else {
            remove_files(&tmp);
        }
    }
    if !done && let Some(s) = &side {
        match restore_backup(s, &tmp, opts, &mut rep) {
            Ok(true) => {
                rep.method = "backup".into();
                done = true;
            }
            Ok(false) => {}
            Err(e) => rep.notes.push(crate::i18n::error(&e)),
        }
        if !done {
            remove_files(&tmp);
        }
    }
    if !done && let Some(b) = &broken {
        match salvage(b, &tmp, opts, &mut rep) {
            Ok(()) => {
                rep.method = "salvage".into();
                done = true;
            }
            Err(e) => rep.notes.push(crate::i18n::error(&e)),
        }
    }
    let ok = done && open_rw(&tmp, opts).ok().and_then(|c| integrity_check(&c).ok()).is_some_and(|p| is_ok(&p));
    if !ok {
        remove_files(&tmp);
        // Put the damaged file back: nothing has changed.
        if let Some(b) = &broken {
            let _ = move_files(b, path);
        }
        let why = rep.notes.last().cloned().unwrap_or_else(|| problems.first().cloned().unwrap_or_default());
        return Err(err!("sqlite.repairFailed", path.to_string_lossy(), why));
    }
    rename(&tmp, path)?;
    rep.status = "repaired".into();
    let conn = open_rw(path, opts)?;
    rep.tables = table_counts(&conn);
    // Start a new generation from the repaired database.
    if let Some(s) = &side
        && opts.keep > 0
    {
        let _ = s.snapshot(&conn, opts.keep);
    }
    Ok(rep)
}

/// Tries to repair the copy at tmp in place: the method that worked.
fn fix_copy(tmp: &Path, opts: &Options) -> Option<&'static str> {
    // Opening it rolls back a journal left behind.
    let conn = open_rw(tmp, opts).ok()?;
    let ok = |c: &Connection| integrity_check(c).ok().is_some_and(|p| is_ok(&p));
    if ok(&conn) {
        return Some("journal");
    }
    if conn.execute_batch("REINDEX").is_ok() && ok(&conn) {
        return Some("reindex");
    }
    if conn.execute_batch("VACUUM").is_ok() && ok(&conn) {
        return Some("vacuum");
    }
    None
}

/// Rebuilds the database at tmp from the newest good backup and the change
/// logs from it on. False when there is no good backup.
fn restore_backup(s: &Sidecar, tmp: &Path, opts: &Options, rep: &mut Report) -> Result<bool> {
    let logs = s.logs();
    for snap in s.snapshots().iter().rev() {
        let good = open_rw(&snap.path, opts).ok().and_then(|c| quick_check(&c).ok()).is_some_and(|p| is_ok(&p));
        if !good {
            continue;
        }
        remove_files(tmp);
        std::fs::copy(&snap.path, tmp).map_err(|e| Error::path("copy", snap.path.to_string_lossy(), e))?;
        let conn = open_rw(tmp, opts)?;
        for (generation, p) in &logs {
            if *generation >= snap.generation {
                replay(&conn, p, rep);
            }
        }
        rep.backup = Some(snap.time.format("%Y-%m-%d %H:%M:%S").to_string());
        return Ok(true);
    }
    Ok(false)
}

/// Replays the transactions of a change log.
fn replay(conn: &Connection, log: &Path, rep: &mut Report) {
    let Ok(data) = std::fs::read(log) else { return };
    let text = String::from_utf8_lossy(&data);
    let lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        let Ok(e) = serde_json::from_str::<LogEntry>(line) else {
            // A line cut short by a crash at the end is not a failure.
            if i + 1 < lines.len() {
                rep.skipped += 1;
            }
            continue;
        };
        if replay_entry(conn, &e).is_ok() {
            rep.replayed += 1;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
            rep.skipped += 1;
        }
    }
}

fn replay_entry(conn: &Connection, e: &LogEntry) -> rusqlite::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    for s in &e.stmts {
        if s.batch {
            conn.execute_batch(&s.sql)?;
            continue;
        }
        let mut st = conn.prepare_cached(&s.sql)?;
        for (i, p) in s.params.iter().enumerate() {
            st.raw_bind_parameter(i + 1, json_to_value(p))?;
        }
        let mut rows = st.raw_query();
        while rows.next()?.is_some() {}
    }
    conn.execute_batch("COMMIT")
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Copies the schema and every readable row of the damaged database src
/// into a new database at dst.
fn salvage(src: &Path, dst: &Path, opts: &Options, rep: &mut Report) -> Result<()> {
    let from = open_rw(src, opts)?;
    remove_files(dst);
    let to = connect(dst, false, true, false, opts.timeout)?;
    to.execute_batch("PRAGMA foreign_keys=OFF").map_err(sql_error)?;
    let schema: Vec<(String, String, String)> = {
        let mut st = from
            .prepare("SELECT type, name, sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, rowid")
            .map_err(sql_error)?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql_error)?;
        rows.collect::<rusqlite::Result<_>>().map_err(sql_error)?
    };
    // Tables first, then copy the rows, then indexes, triggers and views.
    let mut tables = Vec::new();
    for (ty, name, sql) in &schema {
        if ty != "table" {
            continue;
        }
        if name.starts_with("sqlite_") {
            continue;
        }
        match to.execute_batch(sql) {
            Ok(()) => tables.push(name.clone()),
            Err(e) => rep.notes.push(format!("{name}: {}", crate::i18n::error(&sql_error(e)))),
        }
    }
    for name in &tables {
        copy_rows(&from, &to, name, rep);
    }
    // AUTOINCREMENT counters.
    if let Ok(mut st) = from.prepare("SELECT name, seq FROM sqlite_sequence") {
        let seqs: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.flatten().collect()).unwrap_or_default();
        for (n, s) in seqs {
            let _ = to.execute("UPDATE sqlite_sequence SET seq = max(seq, ?2) WHERE name = ?1", rusqlite::params![n, s]);
        }
    }
    for (ty, name, sql) in &schema {
        if ty == "table" {
            continue;
        }
        if let Err(e) = to.execute_batch(sql) {
            rep.notes.push(format!("{name}: {}", crate::i18n::error(&sql_error(e))));
        }
    }
    Ok(())
}

/// Copies the rows of a table that can be read, stepping over damaged
/// pages by rowid.
fn copy_rows(from: &Connection, to: &Connection, table: &str, rep: &mut Report) {
    let cols: Vec<String> = match to.prepare(&format!("SELECT name FROM pragma_table_xinfo({}) WHERE hidden = 0", sql_lit(table))) {
        Ok(mut st) => st.query_map([], |r| r.get(0)).map(|r| r.flatten().collect()).unwrap_or_default(),
        Err(_) => return,
    };
    if cols.is_empty() {
        return;
    }
    let list = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(",");
    let has_rowid = from.prepare(&format!("SELECT rowid FROM {} LIMIT 0", quote(table))).is_ok();
    let _ = to.execute_batch("BEGIN");
    let mut failed = 0usize;
    let mut read_errors = 0usize;
    if has_rowid {
        let ins = format!("INSERT INTO {}(rowid,{list}) VALUES(?{})", quote(table), ",?".repeat(cols.len()));
        let sel = format!("SELECT rowid,{list} FROM {} WHERE rowid > ? ORDER BY rowid", quote(table));
        let max: Option<i64> = from.query_row(&format!("SELECT max(rowid) FROM {}", quote(table)), [], |r| r.get(0)).ok().flatten();
        let mut last = i64::MIN;
        let mut step = 1i64;
        loop {
            let Ok(mut st) = from.prepare(&sel) else { break };
            let Ok(mut rows) = st.query([last]) else { break };
            let mut progressed = false;
            let mut broke = false;
            loop {
                match rows.next() {
                    Ok(Some(r)) => {
                        let vals: Vec<rusqlite::types::Value> = (0..=cols.len()).map(|i| r.get(i).unwrap_or(rusqlite::types::Value::Null)).collect();
                        if let rusqlite::types::Value::Integer(id) = vals[0] {
                            last = id;
                        }
                        progressed = true;
                        step = 1;
                        if to.execute(&ins, rusqlite::params_from_iter(vals)).is_err() {
                            failed += 1;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        broke = true;
                        break;
                    }
                }
            }
            if !broke {
                break;
            }
            // Jump over the damaged part, further each time.
            read_errors += 1;
            if read_errors > 64 {
                break;
            }
            let base = if last == i64::MIN { 0 } else { last };
            if !progressed {
                step = step.saturating_mul(4);
            }
            last = base.saturating_add(step);
            if max.is_some_and(|m| last >= m) {
                break;
            }
        }
    } else {
        let ins = format!("INSERT INTO {}({list}) VALUES({})", quote(table), vec!["?"; cols.len()].join(","));
        if let Ok(mut st) = from.prepare(&format!("SELECT {list} FROM {}", quote(table)))
            && let Ok(mut rows) = st.query([])
        {
            loop {
                match rows.next() {
                    Ok(Some(r)) => {
                        let vals: Vec<rusqlite::types::Value> = (0..cols.len()).map(|i| r.get(i).unwrap_or(rusqlite::types::Value::Null)).collect();
                        if to.execute(&ins, rusqlite::params_from_iter(vals)).is_err() {
                            failed += 1;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        read_errors += 1;
                        break;
                    }
                }
            }
        }
    }
    let _ = to.execute_batch("COMMIT");
    if read_errors > 0 {
        rep.notes.push(t!("sqlite.salvageDamaged", table));
    }
    if failed > 0 {
        rep.notes.push(t!("sqlite.salvageFailed", table, failed));
    }
}

fn sql_lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The number of rows of each table.
fn table_counts(conn: &Connection) -> Vec<(String, i64)> {
    let names: Vec<String> = match conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name") {
        Ok(mut st) => st.query_map([], |r| r.get(0)).map(|r| r.flatten().collect()).unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    names
        .into_iter()
        .map(|n| {
            let c = conn.query_row(&format!("SELECT count(*) FROM {}", quote(&n)), [], |r| r.get(0)).unwrap_or(-1);
            (n, c)
        })
        .collect()
}
