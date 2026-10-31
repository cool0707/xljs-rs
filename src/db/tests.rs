use std::time::Duration;

use rusqlite::types::Value;

use super::*;
use crate::testutil::{lock_lang, tempdir};

fn opts(shared: bool) -> Options {
    Options { shared: Some(shared), timeout: Duration::from_millis(200), ..Default::default() }
}

fn p(v: Vec<Value>) -> Params {
    Params::Pos(v)
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn count(db: &mut Db, table: &str) -> i64 {
    let r = db.query(&format!("SELECT count(*) FROM {table}"), &p(vec![]), None).unwrap();
    match r.rows[0][0] {
        Value::Integer(n) => n,
        _ => panic!(),
    }
}

/// Fills table t with n rows of some size (several pages).
fn fill(db: &mut Db, n: usize) {
    db.exec("CREATE TABLE IF NOT EXISTS t(id INTEGER PRIMARY KEY, name TEXT, note TEXT)").unwrap();
    let rows = (0..n).map(|i| vec![Value::Integer(i as i64 + 1), text(&format!("name{i}")), text(&"x".repeat(200))]).collect();
    db.insert("t", &["id".into(), "name".into(), "note".into()], rows, "INSERT").unwrap();
}

/// Overwrites the pages from..to (1-based) of the file with garbage.
fn damage(path: &std::path::Path, from: usize, to: usize) {
    let mut data = std::fs::read(path).unwrap();
    let ps = u16::from_be_bytes([data[16], data[17]]) as usize;
    for pg in from..=to {
        let start = (pg - 1) * ps;
        if start >= data.len() {
            break;
        }
        let end = (start + ps).min(data.len());
        for (i, b) in data[start..end].iter_mut().enumerate() {
            *b = (i * 7 + 13) as u8;
        }
    }
    std::fs::write(path, data).unwrap();
}

#[test]
fn local_queries_and_parameters() {
    let _g = lock_lang("en");
    let dir = tempdir();
    let path = dir.join("a.db");
    let mut db = Db::open(path.to_str().unwrap(), opts(false)).unwrap();
    assert!(!db.shared());
    db.exec("CREATE TABLE t(a INTEGER, b TEXT, d DATE)").unwrap();
    let (changes, id) = db.run("INSERT INTO t VALUES(?, ?, ?)", &p(vec![Value::Integer(1), text("x"), text("2026-04-01")])).unwrap();
    assert_eq!((changes, id), (1, 1));
    db.run("INSERT INTO t(a, b) VALUES(:a, @b)", &Params::Named(vec![("a".into(), Value::Integer(2)), ("@b".into(), text("y"))])).unwrap();
    let r = db.query("SELECT * FROM t ORDER BY a", &p(vec![]), None).unwrap();
    assert_eq!(r.columns, ["a", "b", "d"]);
    assert_eq!(r.decl[2].as_deref(), Some("DATE"));
    assert_eq!(r.rows.len(), 2);
    let one = db.query("SELECT b FROM t ORDER BY a", &p(vec![]), Some(1)).unwrap();
    assert_eq!(one.rows, vec![vec![text("x")]]);
    let e = db.run("INSERT INTO t VALUES(?, ?, ?)", &p(vec![Value::Integer(1)])).unwrap_err();
    assert!(e.to_string().contains("3"), "{e}");
    let e = db.run("INSERT INTO t(a) VALUES(:zz)", &Params::Named(vec![])).unwrap_err();
    assert!(e.to_string().contains(":zz"), "{e}");
    // Nested transactions are savepoints.
    db.begin().unwrap();
    db.run("INSERT INTO t(a) VALUES(3)", &p(vec![])).unwrap();
    db.begin().unwrap();
    db.run("INSERT INTO t(a) VALUES(4)", &p(vec![])).unwrap();
    db.rollback().unwrap();
    db.commit().unwrap();
    assert_eq!(count(&mut db, "t"), 3);
    assert!(is_ok(&db.check().unwrap()));
    db.close();
    assert!(db.run("SELECT 1", &p(vec![])).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shared_mode_backs_up_logs_and_locks() {
    let _g = lock_lang("en");
    let dir = tempdir();
    let path = dir.join("s.db");
    let ps = path.to_str().unwrap();
    let mut a = Db::open(ps, opts(true)).unwrap();
    assert!(a.shared());
    let side = Sidecar::find(&path).unwrap();
    assert!(side.dir.join("README.txt").exists());
    fill(&mut a, 10);
    a.run("UPDATE t SET name = ? WHERE id = ?", &p(vec![text("changed"), Value::Integer(3)])).unwrap();
    // A read does not log.
    a.query("SELECT * FROM t", &p(vec![]), None).unwrap();
    assert_eq!(side.snapshots().len(), 1);
    let logs = side.logs();
    assert_eq!(logs.len(), 1);
    let log = std::fs::read_to_string(&logs[0].1).unwrap();
    assert_eq!(log.lines().count(), 3, "{log}");

    // Another user opening the database is shared too, and waits for the
    // writer.
    let mut b = Db::open(ps, Options { shared: None, ..opts(false) }).unwrap();
    assert!(b.shared());
    a.begin().unwrap();
    let e = b.query("SELECT count(*) FROM t", &p(vec![]), None).unwrap_err();
    assert_eq!(e.key(), Some("sqlite.busy"), "{e}");
    a.commit().unwrap();
    assert_eq!(count(&mut b, "t"), 10);

    // A rolled back transaction is not logged.
    a.begin().unwrap();
    a.run("DELETE FROM t", &p(vec![])).unwrap();
    a.rollback().unwrap();
    assert_eq!(std::fs::read_to_string(&logs[0].1).unwrap().lines().count(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shared_repair_restores_backup_and_replays_the_log() {
    let _g = lock_lang("en");
    let dir = tempdir();
    let path = dir.join("s.db");
    let ps = path.to_str().unwrap();
    let mut db = Db::open(ps, opts(true)).unwrap();
    fill(&mut db, 300);
    db.run("UPDATE t SET name = 'last' WHERE id = 300", &p(vec![])).unwrap();
    db.close();
    damage(&path, 2, 200);
    let mut db = Db::open(ps, opts(true)).unwrap();
    let e = db.query("SELECT * FROM t", &p(vec![]), None).unwrap_err();
    assert!(is_corrupt(&e) && db.corrupt, "{e}");
    let r = db.repair(false).unwrap();
    assert_eq!(r.status, "repaired");
    assert_eq!(r.method, "backup", "{r:?}");
    assert_eq!(r.replayed, 3);
    assert_eq!(r.tables, vec![("t".to_string(), 300)]);
    assert!(std::path::Path::new(r.broken.as_deref().unwrap()).exists());
    let last = db.query("SELECT name FROM t WHERE id = 300", &p(vec![]), None).unwrap();
    assert_eq!(last.rows[0][0], text("last"));
    // A fresh backup of the repaired database.
    assert_eq!(Sidecar::find(&path).unwrap().snapshots().len(), 2);
    // Healthy now: nothing to do.
    assert_eq!(db.repair(false).unwrap().status, "ok");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn local_repair_salvages_readable_rows() {
    let _g = lock_lang("en");
    let dir = tempdir();
    let path = dir.join("l.db");
    let ps = path.to_str().unwrap();
    let mut db = Db::open(ps, opts(false)).unwrap();
    fill(&mut db, 400);
    db.exec("CREATE INDEX t_name ON t(name)").unwrap();
    db.close();
    // A leaf page in the middle of the table.
    let pages = std::fs::metadata(&path).unwrap().len() as usize / 4096;
    damage(&path, pages / 3, pages / 3);
    let r = repair(&path, false, &opts(false)).unwrap();
    assert_eq!(r.status, "repaired");
    let rows = r.tables[0].1;
    assert!(r.method == "salvage" || r.method == "reindex", "{r:?}");
    if r.method == "salvage" {
        assert!(rows > 300 && rows < 400, "{r:?}");
    }
    let mut db = Db::open(ps, opts(false)).unwrap();
    assert!(is_ok(&db.check().unwrap()));
    // The index came back too.
    let idx = db.query("SELECT name FROM sqlite_schema WHERE type = 'index'", &p(vec![]), None).unwrap();
    assert_eq!(idx.rows.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn damaged_index_is_rebuilt_without_loss() {
    let _g = lock_lang("en");
    let dir = tempdir();
    let path = dir.join("i.db");
    let ps = path.to_str().unwrap();
    let mut db = Db::open(ps, opts(false)).unwrap();
    fill(&mut db, 200);
    db.exec("CREATE INDEX t_name ON t(name)").unwrap();
    let root: i64 = match db.query("SELECT rootpage FROM sqlite_schema WHERE name = 't_name'", &p(vec![]), None).unwrap().rows[0][0] {
        Value::Integer(n) => n,
        _ => panic!(),
    };
    db.close();
    damage(&path, root as usize, root as usize);
    let r = repair(&path, false, &opts(false)).unwrap();
    // REINDEX cannot clear a garbage page, so the rows are salvaged; none
    // are lost since only the index was damaged.
    assert!(r.method == "reindex" || r.method == "salvage", "{r:?}");
    assert_eq!(r.tables, vec![("t".to_string(), 200)]);
    let _ = std::fs::remove_dir_all(&dir);
}
