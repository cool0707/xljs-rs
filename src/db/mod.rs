//! SQLite databases (the `sqlite` global of scripts), including databases
//! that several people use from a file server.
//!
//! A database is used in one of two modes:
//!
//! - **local**: one connection stays open until close(), like any SQLite
//!   program.
//! - **shared** (a database on a file server, or one made shared with
//!   `{ shared: true }`; see share.rs for the folder next to it): every
//!   call opens the file, takes the lock file (shared for reading,
//!   exclusive for writing), does its work and closes the file again, so
//!   no client keeps cached pages or locks while the script waits for
//!   people. The journal stays in DELETE mode (WAL cannot work over a
//!   network) with full syncs, and pages are checked as they are read.
//!   Before the first write of each backup interval the database is checked
//!   (quick_check) and backed up, and each committed transaction is
//!   appended to the backup's change log, so that repair() can rebuild the
//!   database from the newest good backup with little or no loss.
//!
//! See docs/sqlite-design.md.

pub mod lock;
mod repair;
pub mod share;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};

pub use repair::{Report, check_file, repair};
use share::{LogStmt, Sidecar};

use crate::{Error, Result, err};

/// How a database is opened.
#[derive(Clone, Debug)]
pub struct Options {
    /// None: shared when the database is already shared or lies on a
    /// network drive.
    pub shared: Option<bool>,
    pub readonly: bool,
    /// Create the file when it is missing.
    pub create: bool,
    /// How long to wait for other people's locks.
    pub timeout: Duration,
    /// Shared mode: back up at most this often (before a write).
    pub interval: Duration,
    /// Shared mode: the number of backups to keep (0 = no backups and no
    /// change log).
    pub keep: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            shared: None,
            readonly: false,
            create: true,
            timeout: Duration::from_secs(30),
            interval: Duration::from_secs(60 * 60),
            keep: 10,
        }
    }
}

/// The values for the parameters of a statement.
pub enum Params {
    /// ?, ?1 ... by position.
    Pos(Vec<Value>),
    /// :name, @name, $name by name (the key with or without the prefix).
    Named(Vec<(String, Value)>),
}

/// The result of a query.
#[derive(Debug)]
pub struct Rows {
    pub columns: Vec<String>,
    /// The declared types of the columns (None for expressions).
    pub decl: Vec<Option<String>>,
    pub rows: Vec<Vec<Value>>,
}

/// An open write transaction (in shared mode with its own connection and
/// the write lock).
struct Txn {
    conn: Option<Connection>,
    _lock: Option<lock::Lock>,
    /// The statements that changed something, for the change log.
    log: Vec<LogStmt>,
    /// The length of log at each savepoint.
    marks: Vec<usize>,
}

pub struct Db {
    pub path: PathBuf,
    pub opts: Options,
    side: Option<Sidecar>,
    /// Local mode: the connection.
    conn: Option<Connection>,
    txn: Option<Txn>,
    /// Whether the last failure was a damaged database.
    pub corrupt: bool,
    pub closed: bool,
}

/// The message of a SQLite error (without rusqlite's decorations).
pub(crate) fn sql_error(e: rusqlite::Error) -> Error {
    match e {
        rusqlite::Error::SqliteFailure(f, Some(m)) => {
            if matches!(f.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) {
                return corrupt(m);
            }
            Error::msg(m)
        }
        rusqlite::Error::SqliteFailure(f, None) => {
            let m = rusqlite::ffi::code_to_str(f.extended_code).to_string();
            if matches!(f.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) {
                return corrupt(m);
            }
            Error::msg(m)
        }
        rusqlite::Error::SqlInputError { msg, .. } => Error::msg(msg),
        e => Error::msg(e.to_string()),
    }
}

/// Whether the error says the database is damaged.
pub fn is_corrupt(e: &Error) -> bool {
    e.key() == Some("sqlite.corrupt")
}

fn corrupt(detail: String) -> Error {
    Error::localized("sqlite.corrupt", crate::t!("sqlite.corrupt", detail))
}

pub(crate) fn corrupt_error(problems: &[String]) -> Error {
    corrupt(problems.first().cloned().unwrap_or_default())
}

pub(crate) fn is_ok(problems: &[String]) -> bool {
    problems.len() == 1 && problems[0] == "ok"
}

pub(crate) fn quick_check(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut st = conn.prepare("PRAGMA quick_check(20)")?;
    st.query_map([], |r| r.get::<_, String>(0))?.collect()
}

pub(crate) fn integrity_check(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut st = conn.prepare("PRAGMA integrity_check(20)")?;
    st.query_map([], |r| r.get::<_, String>(0))?.collect()
}

/// Whether path lies on a network drive (a UNC path or a mapped drive).
#[cfg(windows)]
pub fn is_network(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows::core::PCWSTR;
    let s = path.to_string_lossy().replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return rest.to_ascii_uppercase().starts_with(r"UNC\");
    }
    if s.starts_with(r"\\") {
        return true;
    }
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        let root: Vec<u16> = format!("{}:\\", b[0] as char).encode_utf16().chain([0]).collect();
        // DRIVE_REMOTE
        return unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == 4;
    }
    false
}

#[cfg(not(windows))]
pub fn is_network(_path: &Path) -> bool {
    false
}

/// Opens a connection to path with the settings xljs uses.
pub(crate) fn connect(path: &Path, readonly: bool, create: bool, shared: bool, timeout: Duration) -> Result<Connection> {
    let mut flags = OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if readonly {
        flags |= OpenFlags::SQLITE_OPEN_READ_ONLY;
    } else {
        flags |= OpenFlags::SQLITE_OPEN_READ_WRITE;
        if create {
            flags |= OpenFlags::SQLITE_OPEN_CREATE;
        }
    }
    let conn = Connection::open_with_flags(path, flags).map_err(|e| sql_error(e).context(path.to_string_lossy()))?;
    conn.busy_timeout(timeout).map_err(sql_error)?;
    let mut pragmas = String::from("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF;");
    if shared {
        // Sync the journal's folder too, and check every page as it is
        // read so that damage is noticed before it spreads.
        pragmas.push_str(" PRAGMA synchronous=EXTRA; PRAGMA cell_size_check=ON;");
    }
    conn.execute_batch(&pragmas).map_err(sql_error)?;
    Ok(conn)
}

impl Db {
    /// Opens (or creates) the database at path.
    pub fn open(path: &str, opts: Options) -> Result<Db> {
        let path = std::path::absolute(path).map_err(|e| Error::path("open", path, e))?;
        let exists = path.exists();
        if !exists && (!opts.create || opts.readonly) {
            return Err(Error::path("open", path.to_string_lossy(), std::io::Error::from(std::io::ErrorKind::NotFound)));
        }
        let mut side = Sidecar::find(&path);
        if side.is_none() && opts.shared.unwrap_or_else(|| is_network(&path)) {
            match Sidecar::create(&path) {
                Ok(s) => side = Some(s),
                // Read-only use of a folder we cannot write to: there is
                // nobody to lock out.
                Err(_) if opts.readonly => {}
                Err(e) => return Err(e),
            }
        }
        let mut db = Db { path, opts, side, conn: None, txn: None, corrupt: false, closed: false };
        if let Some(side) = &db.side {
            let _lock = side.lock(&db.path, !db.opts.readonly, db.opts.timeout)?;
            let conn = db.connect()?;
            if !db.opts.readonly {
                // WAL needs shared memory, which does not work over a
                // network: use a rollback journal.
                let r = conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0));
                let r = match r {
                    Ok(mode) if mode.eq_ignore_ascii_case("wal") => conn.query_row("PRAGMA journal_mode=DELETE", [], |_| Ok(())),
                    r => r.map(|_| ()),
                };
                if let Err(e) = r {
                    let e = db.fail(e);
                    // A damaged database opens; the first call reports it
                    // (and offers to repair it).
                    if !db.corrupt {
                        return Err(e);
                    }
                }
            }
        } else {
            db.conn = Some(db.connect()?);
        }
        Ok(db)
    }

    fn connect(&self) -> Result<Connection> {
        connect(&self.path, self.opts.readonly, self.opts.create, self.side.is_some(), self.opts.timeout)
    }

    pub fn shared(&self) -> bool {
        self.side.is_some()
    }

    pub fn in_transaction(&self) -> bool {
        self.txn.is_some()
    }

    /// The nesting level of transactions (0 outside).
    pub fn depth(&self) -> usize {
        self.txn.as_ref().map(|t| t.marks.len() + 1).unwrap_or(0)
    }

    fn fail(&mut self, e: rusqlite::Error) -> Error {
        let e = sql_error(e);
        if is_corrupt(&e) {
            self.corrupt = true;
        }
        e
    }

    fn note(&mut self, e: Error) -> Error {
        if is_corrupt(&e) {
            self.corrupt = true;
        }
        e
    }

    fn check_open(&self) -> Result<()> {
        if self.closed { Err(err!("sqlite.closed")) } else { Ok(()) }
    }

    /// The connection for statements: the transaction's, or the local
    /// one.
    fn cur(&self) -> &Connection {
        self.txn.as_ref().and_then(|t| t.conn.as_ref()).or(self.conn.as_ref()).expect("no connection")
    }

    /// Begins a transaction (a savepoint when one is open).
    pub fn begin(&mut self) -> Result<()> {
        self.check_open()?;
        self.corrupt = false;
        if let Some(t) = &mut self.txn {
            let n = t.marks.len() + 1;
            t.marks.push(t.log.len());
            let conn = t.conn.as_ref().or(self.conn.as_ref()).expect("no connection");
            if let Err(e) = conn.execute_batch(&format!("SAVEPOINT xljs_{n}")) {
                self.txn.as_mut().unwrap().marks.pop();
                return Err(self.fail(e));
            }
            return Ok(());
        }
        let begin = if self.opts.readonly { "BEGIN" } else { "BEGIN IMMEDIATE" };
        let Some(side) = &self.side else {
            self.conn.as_ref().expect("no connection").execute_batch(begin).map_err(|e| self.fail(e))?;
            self.txn = Some(Txn { conn: None, _lock: None, log: Vec::new(), marks: Vec::new() });
            return Ok(());
        };
        let lock = side.lock(&self.path, !self.opts.readonly, self.opts.timeout)?;
        let conn = self.connect().map_err(|e| self.note(e))?;
        if !self.opts.readonly && self.opts.keep > 0 {
            let side = self.side.as_ref().unwrap();
            if let Err(e) = side.maybe_snapshot(&conn, self.opts.interval, self.opts.keep) {
                return Err(self.note(e));
            }
        }
        conn.execute_batch(begin).map_err(|e| self.fail(e))?;
        self.txn = Some(Txn { conn: Some(conn), _lock: Some(lock), log: Vec::new(), marks: Vec::new() });
        Ok(())
    }

    /// Commits the innermost transaction. When it fails the transaction
    /// stays open (call rollback).
    pub fn commit(&mut self) -> Result<()> {
        self.check_open()?;
        let Some(t) = &mut self.txn else { return Err(err!("sqlite.noTransaction")) };
        if let n @ 1.. = t.marks.len() {
            let conn = t.conn.as_ref().or(self.conn.as_ref()).expect("no connection");
            conn.execute_batch(&format!("RELEASE xljs_{n}")).map_err(|e| self.fail(e))?;
            self.txn.as_mut().unwrap().marks.pop();
            return Ok(());
        }
        let conn = t.conn.as_ref().or(self.conn.as_ref()).expect("no connection");
        conn.execute_batch("COMMIT").map_err(|e| self.fail(e))?;
        let t = self.txn.take().unwrap();
        if let Some(side) = &self.side
            && self.opts.keep > 0
            && !t.log.is_empty()
        {
            // The data is committed; a log that cannot be written only
            // means a bigger loss should the database ever be repaired.
            let _ = side.append_log(t.log);
        }
        Ok(())
    }

    /// Rolls back the innermost transaction.
    pub fn rollback(&mut self) -> Result<()> {
        self.check_open()?;
        let Some(t) = &mut self.txn else { return Ok(()) };
        if let Some(mark) = t.marks.pop() {
            let n = t.marks.len() + 1;
            t.log.truncate(mark);
            let conn = t.conn.as_ref().or(self.conn.as_ref()).expect("no connection");
            conn.execute_batch(&format!("ROLLBACK TO xljs_{n}; RELEASE xljs_{n}")).map_err(|e| self.fail(e))?;
            return Ok(());
        }
        let t = self.txn.take().unwrap();
        let conn = t.conn.as_ref().or(self.conn.as_ref()).expect("no connection");
        // Fails when SQLite already rolled back (after some errors).
        let _ = conn.execute_batch("ROLLBACK");
        Ok(())
    }

    /// Runs f in a transaction of its own unless one is open.
    fn write<T>(&mut self, f: impl FnOnce(&mut Db) -> Result<T>) -> Result<T> {
        if self.txn.is_some() {
            return f(self);
        }
        self.begin()?;
        match f(self) {
            Ok(v) => match self.commit() {
                Ok(()) => Ok(v),
                Err(e) => {
                    let _ = self.rollback();
                    Err(e)
                }
            },
            Err(e) => {
                let _ = self.rollback();
                Err(e)
            }
        }
    }

    /// Runs f with a connection for reading.
    fn read<T>(&mut self, f: impl FnOnce(&mut Db, &Connection) -> Result<T>) -> Result<T> {
        self.check_open()?;
        self.corrupt = false;
        if self.txn.is_some() || self.side.is_none() {
            // f only uses the connection through the argument.
            let conn: *const Connection = self.cur();
            return f(self, unsafe { &*conn });
        }
        let side = self.side.as_ref().unwrap();
        let _lock = side.lock(&self.path, false, self.opts.timeout)?;
        let conn = self.connect().map_err(|e| self.note(e))?;
        f(self, &conn)
    }

    /// Runs a query: its rows (at most limit).
    pub fn query(&mut self, sql: &str, params: &Params, limit: Option<usize>) -> Result<Rows> {
        // Statements that change something (INSERT ... RETURNING) need the
        // write lock and go into the change log.
        let ro = self.read(|db, conn| {
            let st = conn.prepare(sql).map_err(|e| db.fail(e))?;
            if !st.readonly() {
                return Ok(None);
            }
            let mut st = st;
            run_query(&mut st, params, limit).map(Some).map_err(|e| db.note(e))
        })?;
        if let Some(r) = ro {
            return Ok(r.0);
        }
        self.write(|db| {
            let conn: *const Connection = db.cur();
            let mut st = unsafe { &*conn }.prepare(sql).map_err(|e| db.fail(e))?;
            let (rows, bound) = run_query(&mut st, params, limit).map_err(|e| db.note(e))?;
            db.log(sql, bound, false);
            Ok(rows)
        })
    }

    /// Runs a statement: (changes, last insert rowid).
    pub fn run(&mut self, sql: &str, params: &Params) -> Result<(u64, i64)> {
        self.check_open()?;
        self.write(|db| {
            let conn: *const Connection = db.cur();
            let conn = unsafe { &*conn };
            let mut st = conn.prepare(sql).map_err(|e| db.fail(e))?;
            let ro = st.readonly();
            let bound = bind(&mut st, params)?;
            let mut rows = st.raw_query();
            loop {
                match rows.next() {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(e) => return Err(db.fail(e)),
                }
            }
            drop(rows);
            if !ro {
                db.log(sql, bound, false);
            }
            Ok((conn.changes(), conn.last_insert_rowid()))
        })
    }

    /// Runs several statements separated by semicolons (without values).
    pub fn exec(&mut self, sql: &str) -> Result<()> {
        self.check_open()?;
        self.write(|db| {
            let conn: *const Connection = db.cur();
            unsafe { &*conn }.execute_batch(sql).map_err(|e| db.fail(e))?;
            db.log(sql, Vec::new(), true);
            Ok(())
        })
    }

    /// Inserts rows (values in the order of columns) in one transaction:
    /// the number of rows.
    pub fn insert(&mut self, table: &str, columns: &[String], rows: Vec<Vec<Value>>, verb: &str) -> Result<usize> {
        self.check_open()?;
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let sql = format!(
            "{verb} INTO {}({}) VALUES({})",
            q(table),
            columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(","),
            vec!["?"; columns.len()].join(",")
        );
        let n = rows.len();
        self.write(|db| {
            let conn: *const Connection = db.cur();
            let mut st = unsafe { &*conn }.prepare(&sql).map_err(|e| db.fail(e))?;
            for row in rows {
                let p = Params::Pos(row);
                let bound = bind(&mut st, &p)?;
                st.raw_execute().map_err(|e| db.fail(e))?;
                db.log(&sql, bound, false);
            }
            Ok(n)
        })
    }

    fn log(&mut self, sql: &str, bound: Vec<Value>, batch: bool) {
        if self.side.is_none() || self.opts.keep == 0 {
            return;
        }
        if let Some(t) = &mut self.txn {
            t.log.push(LogStmt { sql: sql.to_string(), params: bound.iter().map(share::value_to_json).collect(), batch });
        }
    }

    /// PRAGMA integrity_check: ["ok"] or the problems found.
    pub fn check(&mut self) -> Result<Vec<String>> {
        self.read(|db, conn| integrity_check(conn).map_err(|e| db.fail(e)))
    }

    /// Copies the database to dest (a consistent copy while it is in use).
    pub fn backup(&mut self, dest: &str) -> Result<()> {
        self.read(|db, conn| conn.backup(rusqlite::MAIN_DB, dest, None::<fn(rusqlite::backup::Progress)>).map_err(|e| db.fail(e).context(dest.to_string())))
    }

    /// Repairs the database (see repair.rs), closing it meanwhile.
    pub fn repair(&mut self, force: bool) -> Result<Report> {
        self.check_open()?;
        if self.txn.is_some() {
            return Err(err!("sqlite.repairInTransaction"));
        }
        self.conn = None;
        let r = repair(&self.path, force, &self.opts);
        self.corrupt = false;
        if self.side.is_none() {
            self.side = Sidecar::find(&self.path);
        }
        if self.side.is_none() {
            self.conn = Some(self.connect()?);
        }
        r
    }

    /// Closes the database; an open transaction is rolled back.
    pub fn close(&mut self) {
        if let Some(t) = self.txn.take()
            && let Some(c) = t.conn.as_ref().or(self.conn.as_ref())
        {
            let _ = c.execute_batch("ROLLBACK");
        }
        self.conn = None;
        self.closed = true;
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        self.close();
    }
}

/// Binds params to the statement: the values by position (for the log).
fn bind(st: &mut rusqlite::Statement, params: &Params) -> Result<Vec<Value>> {
    let n = st.parameter_count();
    let mut out = Vec::with_capacity(n);
    match params {
        Params::Pos(v) => {
            if v.len() != n {
                return Err(err!("sqlite.paramCount", n, v.len()));
            }
            out.extend(v.iter().cloned());
        }
        Params::Named(m) => {
            for i in 1..=n {
                let Some(name) = st.parameter_name(i) else {
                    return Err(err!("sqlite.paramPositional"));
                };
                let bare = name.trim_start_matches([':', '@', '$', '?']);
                let v = m.iter().find(|(k, _)| k == name || k == bare).map(|(_, v)| v.clone());
                let Some(v) = v else { return Err(err!("sqlite.paramMissing", name)) };
                out.push(v);
            }
        }
    }
    for (i, v) in out.iter().enumerate() {
        st.raw_bind_parameter(i + 1, v).map_err(sql_error)?;
    }
    Ok(out)
}

/// Runs a query statement: the rows and the values bound.
fn run_query(st: &mut rusqlite::Statement, params: &Params, limit: Option<usize>) -> Result<(Rows, Vec<Value>)> {
    let bound = bind(st, params)?;
    let columns: Vec<String> = st.column_names().map(String::from).collect();
    let decl: Vec<Option<String>> = st.columns().map(|c| c.decl_type().map(String::from)).collect();
    let n = columns.len();
    let mut out = Vec::new();
    let mut rows = st.raw_query();
    while limit.is_none_or(|l| out.len() < l) {
        let Some(r) = rows.next().map_err(sql_error)? else { break };
        let mut vals = Vec::with_capacity(n);
        for i in 0..n {
            vals.push(r.get::<_, Value>(i).map_err(sql_error)?);
        }
        out.push(vals);
    }
    Ok((Rows { columns, decl, rows: out }, bound))
}

#[cfg(test)]
mod tests;
