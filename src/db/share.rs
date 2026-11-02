//! The folder next to a shared database (`<file>.xljs-shared`): the lock
//! file, who wrote last, the backups (snapshots) and the change logs that
//! repair() replays on top of the newest good snapshot.
//!
//! ```text
//! 顧客.db
//! 顧客.db.xljs-shared/
//!   README.txt                     what the folder is for
//!   lock                           the reader-writer lock (see lock.rs)
//!   owner.json                     who started the last write
//!   backups/snap-000012-20261031-093000.db
//!   backups/log-000012.jsonl       transactions committed after snapshot 12
//!   broken/                        damaged files put aside by repair()
//! ```

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use chrono::{Local, NaiveDateTime};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use serde_json::Value as J;

use super::lock::{self, Lock, LockError};
use crate::{Error, Result, err, t};

pub const SUFFIX: &str = ".xljs-shared";
const STAMP: &str = "%Y%m%d-%H%M%S";

pub struct Sidecar {
    pub dir: PathBuf,
}

/// A backup of the database.
#[derive(Clone, Debug)]
pub struct Snap {
    pub generation: u64,
    pub time: NaiveDateTime,
    pub path: PathBuf,
}

/// A committed transaction in a change log.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct LogEntry {
    pub time: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub computer: String,
    pub stmts: Vec<LogStmt>,
}

/// A statement of a logged transaction with the values bound to its
/// parameters (by position), or a batch of statements without values.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogStmt {
    pub sql: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<J>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub batch: bool,
}

/// The value as JSON: integers and reals stay apart (1 and 1.0), blobs
/// are {"b": base64}, non-finite reals {"r": "inf"}.
pub fn value_to_json(v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Integer(i) => J::from(*i),
        Value::Real(f) if f.is_finite() => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::Real(f) => serde_json::json!({"r": if f.is_nan() { "nan" } else if *f > 0.0 { "inf" } else { "-inf" }}),
        Value::Text(s) => J::String(s.clone()),
        Value::Blob(b) => serde_json::json!({"b": base64::engine::general_purpose::STANDARD.encode(b)}),
    }
}

pub fn json_to_value(j: &J) -> Value {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Integer(*b as i64),
        J::Number(n) => match n.as_i64() {
            Some(i) if !n.is_f64() => Value::Integer(i),
            _ => Value::Real(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => Value::Text(s.clone()),
        J::Object(o) => {
            if let Some(b) = o.get("b").and_then(J::as_str) {
                return Value::Blob(base64::engine::general_purpose::STANDARD.decode(b).unwrap_or_default());
            }
            match o.get("r").and_then(J::as_str) {
                Some("inf") => Value::Real(f64::INFINITY),
                Some("-inf") => Value::Real(f64::NEG_INFINITY),
                _ => Value::Null,
            }
        }
        J::Array(_) => Value::Text(j.to_string()),
    }
}

/// Who is running this: (user, computer).
pub fn whoami() -> (String, String) {
    let get = |names: &[&str]| names.iter().find_map(|n| std::env::var(n).ok().filter(|s| !s.is_empty())).unwrap_or_default();
    (get(&["USERNAME", "USER"]), get(&["COMPUTERNAME", "HOSTNAME"]))
}

fn now_text() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

impl Sidecar {
    pub fn path_for(db: &Path) -> PathBuf {
        let name = db.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        db.with_file_name(format!("{name}{SUFFIX}"))
    }

    /// The folder of a database that is shared, if it is.
    pub fn find(db: &Path) -> Option<Sidecar> {
        let dir = Self::path_for(db);
        dir.is_dir().then_some(Sidecar { dir })
    }

    /// Makes the database shared (creates the folder).
    pub fn create(db: &Path) -> Result<Sidecar> {
        let dir = Self::path_for(db);
        let s = Sidecar { dir };
        for d in [&s.dir, &s.backups_dir()] {
            std::fs::create_dir_all(d).map_err(|e| Error::path("mkdir", d.to_string_lossy(), e))?;
        }
        let readme = s.dir.join("README.txt");
        if !readme.exists() {
            let name = db.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let text = t!("sqlite.readme", name).replace('\n', "\r\n");
            let _ = std::fs::write(&readme, text);
        }
        lock::ensure(&s.lock_path()).map_err(|e| Error::path("open", s.lock_path().to_string_lossy(), e))?;
        Ok(s)
    }

    pub fn lock_path(&self) -> PathBuf {
        self.dir.join("lock")
    }
    fn owner_path(&self) -> PathBuf {
        self.dir.join("owner.json")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.dir.join("backups")
    }
    pub fn broken_dir(&self) -> PathBuf {
        self.dir.join("broken")
    }

    /// Takes the lock of the database db (for the messages).
    pub fn lock(&self, db: &Path, write: bool, timeout: Duration) -> Result<Lock> {
        match lock::acquire(&self.lock_path(), write, timeout) {
            Ok(l) => {
                if write {
                    self.write_owner();
                }
                Ok(l)
            }
            Err(LockError::Io(e)) => Err(Error::path("open", self.lock_path().to_string_lossy(), e)),
            Err(LockError::Busy) => {
                let who = self.owner().map(|(u, c, time)| t!("sqlite.lastWriter", u, c, time)).unwrap_or_default();
                Err(err!("sqlite.busy", db.to_string_lossy(), who))
            }
        }
    }

    fn write_owner(&self) {
        let (user, computer) = whoami();
        let j = serde_json::json!({"user": user, "computer": computer, "time": now_text()});
        let _ = std::fs::write(self.owner_path(), j.to_string());
    }

    /// (user, computer, time) of the last write.
    fn owner(&self) -> Option<(String, String, String)> {
        let j: J = serde_json::from_slice(&std::fs::read(self.owner_path()).ok()?).ok()?;
        let s = |k: &str| j.get(k).and_then(J::as_str).unwrap_or("").to_string();
        Some((s("user"), s("computer"), s("time")))
    }

    /// The backups, oldest first.
    pub fn snapshots(&self) -> Vec<Snap> {
        let mut v: Vec<Snap> = std::fs::read_dir(self.backups_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let rest = name.strip_prefix("snap-")?.strip_suffix(".db")?;
                let (generation, time) = rest.split_once('-')?;
                Some(Snap { generation: generation.parse().ok()?, time: NaiveDateTime::parse_from_str(time, STAMP).ok()?, path: e.path() })
            })
            .collect();
        v.sort_by_key(|s| s.generation);
        v
    }

    /// The change logs (generation, path), oldest first.
    pub fn logs(&self) -> Vec<(u64, PathBuf)> {
        let mut v: Vec<(u64, PathBuf)> = std::fs::read_dir(self.backups_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let generation = name.strip_prefix("log-")?.strip_suffix(".jsonl")?.parse().ok()?;
                Some((generation, e.path()))
            })
            .collect();
        v.sort_by_key(|x| x.0);
        v
    }

    fn log_path(&self, generation: u64) -> PathBuf {
        self.backups_dir().join(format!("log-{generation:06}.jsonl"))
    }

    /// Takes a backup of the database open in conn when there is none yet
    /// or the newest is older than interval. The database is checked
    /// first: a damaged one is not backed up (the error says it is
    /// damaged). The caller holds the write lock.
    pub fn maybe_snapshot(&self, conn: &rusqlite::Connection, interval: Duration, keep: usize) -> Result<()> {
        let newest = self.snapshots().pop();
        if let Some(s) = &newest {
            let age = Local::now().naive_local() - s.time;
            if age >= chrono::TimeDelta::zero() && age.to_std().unwrap_or_default() < interval {
                return Ok(());
            }
        }
        let problems = super::quick_check(conn).map_err(super::sql_error)?;
        if !super::is_ok(&problems) {
            return Err(super::corrupt_error(&problems));
        }
        self.snapshot(conn, keep)?;
        Ok(())
    }

    /// Backs up the database open in conn as a new generation, whose log
    /// starts empty, and removes the backups and logs beyond keep.
    pub fn snapshot(&self, conn: &rusqlite::Connection, keep: usize) -> Result<Snap> {
        let dir = self.backups_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::path("mkdir", dir.to_string_lossy(), e))?;
        let generation = self.snapshots().last().map(|s| s.generation).max(self.logs().last().map(|l| l.0)).unwrap_or(0) + 1;
        let now = Local::now().naive_local();
        let tmp = dir.join(format!("snap-{generation:06}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        conn.backup(rusqlite::MAIN_DB, &tmp, None::<fn(rusqlite::backup::Progress)>).map_err(super::sql_error)?;
        let path = dir.join(format!("snap-{generation:06}-{}.db", now.format(STAMP)));
        std::fs::rename(&tmp, &path).map_err(|e| Error::path("rename", tmp.to_string_lossy(), e))?;
        self.prune(keep.max(1));
        Ok(Snap { generation, time: now, path })
    }

    /// Keeps the newest keep backups and the logs from the oldest of them on.
    fn prune(&self, keep: usize) {
        let snaps = self.snapshots();
        if snaps.len() > keep {
            for s in &snaps[..snaps.len() - keep] {
                let _ = std::fs::remove_file(&s.path);
            }
        }
        let first = snaps.len().saturating_sub(keep);
        if let Some(oldest) = snaps.get(first) {
            for (generation, p) in self.logs() {
                if generation < oldest.generation {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }

    /// Appends a committed transaction to the log of the newest backup.
    pub fn append_log(&self, stmts: Vec<LogStmt>) -> Result<()> {
        let Some(generation) = self.snapshots().last().map(|s| s.generation) else { return Ok(()) };
        let (user, computer) = whoami();
        let e = LogEntry { time: now_text(), user, computer, stmts };
        let mut line = serde_json::to_string(&e)?;
        line.push('\n');
        let p = self.log_path(generation);
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| Error::path("open", p.to_string_lossy(), e))?;
        // One write for the whole line, flushed through to the server.
        f.write_all(line.as_bytes()).map_err(|e| Error::path("write", p.to_string_lossy(), e))?;
        f.sync_data().map_err(|e| Error::path("write", p.to_string_lossy(), e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_through_json() {
        for v in [
            Value::Null,
            Value::Integer(1),
            Value::Integer(i64::MAX),
            Value::Real(1.0),
            Value::Real(-2.5),
            Value::Real(f64::INFINITY),
            Value::Text("あ".into()),
            Value::Blob(vec![0, 1, 255]),
        ] {
            let j = value_to_json(&v);
            let back = json_to_value(&serde_json::from_str(&j.to_string()).unwrap());
            assert_eq!(back, v, "{j}");
        }
    }
}
