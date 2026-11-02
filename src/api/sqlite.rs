//! SQLite databases for scripts (the natives behind the `sqlite` global;
//! the databases themselves are in crate::db).

use std::sync::LazyLock;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;
use rquickjs::class::Trace;
use rquickjs::{BigInt, Class, Constructor, Ctx, JsLifetime, Value};
use rusqlite::types::Value as Sql;

use super::js::{self, elems, nullish};
use super::{A, Host, N, R};
use crate::db::{self, Db, Params};
use crate::err;

/// The handle of an open database in JavaScript.
#[derive(Trace, JsLifetime, Clone)]
#[rquickjs::class(rename = "SqliteDatabase")]
pub struct Handle {
    #[qjs(skip_trace)]
    pub id: u64,
}

fn handle_id(v: &Value) -> Option<u64> {
    let c = Class::<Handle>::from_object(v.as_object()?)?;
    Some(c.borrow().id)
}

/// Runs f with the open database of the handle (argument 0).
fn with_db<'js, T>(h: &Host, a: &A<'js>, f: impl FnOnce(&mut Db) -> crate::Result<T>) -> rquickjs::Result<T> {
    let Some(id) = handle_id(&a.arg(0)) else { return Err(a.throw(&err!("sqlite.invalidHandle"))) };
    let mut dbs = h.dbs.borrow_mut();
    let Some(d) = dbs.get_mut(&id) else { return Err(a.throw(&err!("sqlite.invalidHandle"))) };
    f(d).map_err(|e| a.throw(&e))
}

// ---- values ----

const MAX_SAFE: i64 = (1 << 53) - 1;

fn pad(n: u32, w: usize) -> String {
    format!("{n:0w$}")
}

/// A Date as SQLite text in local time: "2026-04-01" at midnight,
/// "2026-04-01 09:30:00" (".123" with milliseconds) otherwise.
fn date_text(f: (i32, u32, u32, u32, u32, u32, u32)) -> String {
    let (y, mo, d, h, mi, s, ms) = f;
    let date = format!("{y:04}-{}-{}", pad(mo, 2), pad(d, 2));
    if (h, mi, s, ms) == (0, 0, 0, 0) {
        return date;
    }
    let mut t = format!("{date} {}:{}:{}", pad(h, 2), pad(mi, 2), pad(s, 2));
    if ms > 0 {
        t.push_str(&format!(".{}", pad(ms, 3)));
    }
    t
}

/// A JavaScript value for a parameter or a column.
fn to_sql<'js>(a: &A<'js>, v: &Value<'js>) -> rquickjs::Result<Sql> {
    if nullish(v) {
        return Ok(Sql::Null);
    }
    if let Some(b) = v.as_bool() {
        return Ok(Sql::Integer(b as i64));
    }
    if let Some(n) = v.as_number() {
        if n.is_nan() {
            return Ok(Sql::Null);
        }
        if n.fract() == 0.0 && n.abs() <= MAX_SAFE as f64 && !(n == 0.0 && n.is_sign_negative()) {
            return Ok(Sql::Integer(n as i64));
        }
        return Ok(Sql::Real(n));
    }
    if v.is_string() {
        return Ok(Sql::Text(js::to_str(v)));
    }
    if let Some(b) = v.as_big_int() {
        let s = js::to_str(v);
        return b.clone().to_i64().map(Sql::Integer).map_err(|_| a.throw_t("sqlite.bigint", &[s]));
    }
    if js::is_date(v) {
        return match js::date_fields(v) {
            Some(f) => Ok(Sql::Text(date_text(f))),
            None => Ok(Sql::Null),
        };
    }
    if let Some(o) = v.as_object() {
        let bytesish = o.as_array_buffer().is_some() || (js::has(v, "buffer") && js::has(v, "byteLength") && !v.is_array());
        if bytesish {
            return Ok(Sql::Blob(a_bytes(a, v)?));
        }
        if !v.is_function() {
            // Objects and arrays are stored as JSON.
            if let Some(s) = a.ctx.json_stringify(v.clone())? {
                return Ok(Sql::Text(s.to_string()?));
            }
        }
    }
    Err(a.throw_t("sqlite.badValue", &[v.type_name().to_string()]))
}

fn a_bytes<'js>(a: &A<'js>, v: &Value<'js>) -> rquickjs::Result<Vec<u8>> {
    js::bytes(&a.ctx, v)
}

fn params<'js>(a: &A<'js>, i: usize) -> rquickjs::Result<Params> {
    let v = a.arg(i);
    if nullish(&v) {
        return Ok(Params::Pos(Vec::new()));
    }
    if v.is_array() {
        return elems(&v).iter().map(|e| to_sql(a, e)).collect::<rquickjs::Result<_>>().map(Params::Pos);
    }
    let Some(o) = v.as_object() else { return Ok(Params::Pos(vec![to_sql(a, &v)?])) };
    let mut out = Vec::new();
    for k in o.keys::<String>() {
        let k = k?;
        let x: Value = o.get(&k)?;
        out.push((k, to_sql(a, &x)?));
    }
    Ok(Params::Named(out))
}

static DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?)?\s*(Z|[+-]\d{2}:?\d{2})?$").unwrap()
});

/// Text of a date column as a Date (None when it is not a date).
fn parse_date<'js>(ctx: &Ctx<'js>, s: &str) -> Option<Value<'js>> {
    let c = DATE_RE.captures(s.trim())?;
    let n = |i: usize| c.get(i).map(|m| m.as_str().parse::<u32>().unwrap_or(0)).unwrap_or(0);
    let ms = c.get(7).map(|m| format!("{:0<3}", &m.as_str()[..m.as_str().len().min(3)]).parse::<u32>().unwrap_or(0)).unwrap_or(0);
    let date = NaiveDate::from_ymd_opt(n(1) as i32, n(2), n(3))?;
    let dt: NaiveDateTime = date.and_hms_milli_opt(n(4), n(5), n(6), ms)?;
    match c.get(8).map(|m| m.as_str()) {
        None => js::date_local(ctx, n(1) as i32, n(2), n(3), n(4), n(5), n(6), ms).ok(),
        Some(z) => {
            let off = if z == "Z" {
                0
            } else {
                let sign = if z.starts_with('-') { -1 } else { 1 };
                let digits: String = z[1..].chars().filter(char::is_ascii_digit).collect();
                let (hh, mm) = (digits[..2].parse::<i64>().ok()?, digits[2..].parse::<i64>().ok()?);
                sign * (hh * 60 + mm)
            };
            let utc_ms = dt.and_utc().timestamp_millis() - off * 60_000;
            js::date_ms(ctx, utc_ms as f64).ok()
        }
    }
}

/// A column value as JavaScript: integers beyond 2^53 as BigInt, blobs
/// as Uint8Array, dates (text in a column declared DATE, DATETIME,
/// TIMESTAMP ...) as Date, 0/1 in a BOOLEAN column as false/true.
fn to_js<'js>(ctx: &Ctx<'js>, v: Sql, decl: Option<&str>) -> R<'js> {
    let decl = decl.map(|d| d.to_ascii_uppercase()).unwrap_or_default();
    Ok(match v {
        Sql::Null => Value::new_null(ctx.clone()),
        Sql::Integer(i) if decl.contains("BOOL") && (i == 0 || i == 1) => Value::new_bool(ctx.clone(), i == 1),
        Sql::Integer(i) if i.abs() <= MAX_SAFE => Value::new_number(ctx.clone(), i as f64),
        Sql::Integer(i) => BigInt::from_i64(ctx.clone(), i)?.into_value(),
        Sql::Real(f) => Value::new_number(ctx.clone(), f),
        Sql::Text(s) => {
            if (decl.contains("DATE") || decl.contains("TIME"))
                && let Some(d) = parse_date(ctx, &s)
            {
                return Ok(d);
            }
            js::v(ctx, s)
        }
        Sql::Blob(b) => {
            let ab = js::array_buffer(ctx, b)?;
            let c: Constructor = ctx.globals().get("Uint8Array")?;
            c.construct((ab,))?
        }
    })
}

/// Column names made unique like csv headers do (id, id_2 ...).
fn unique_names(cols: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(cols.len());
    for c in cols {
        let mut name = c.clone();
        let mut n = 2;
        while out.contains(&name) {
            name = format!("{c}_{n}");
            n += 1;
        }
        out.push(name);
    }
    out
}

fn report_json(r: &db::Report) -> String {
    serde_json::to_string(r).unwrap_or_default()
}

fn int_opt(a: &A, i: usize, def: i64) -> i64 {
    if nullish(&a.arg(i)) { def } else { a.int(i) }
}

pub(super) fn install(n: &N) {
    // sqliteOpen(path, shared -1|0|1, readonly, create, timeoutMs, intervalMinutes, keep)
    n.set("sqliteOpen", |h, a| {
        let path = a.str(0);
        let opts = db::Options {
            shared: match int_opt(a, 1, -1) {
                -1 => None,
                0 => Some(false),
                _ => Some(true),
            },
            readonly: a.bool(2),
            create: a.bool(3),
            timeout: Duration::from_millis(int_opt(a, 4, 30_000).max(0) as u64),
            interval: Duration::from_secs((a.f64(5).max(0.0) * 60.0) as u64),
            keep: int_opt(a, 6, 10).max(0) as usize,
        };
        let d = Db::open(&path, opts).map_err(|e| super::throw_path(&a.ctx, &path, e))?;
        let id = h.next_id();
        h.dbs.borrow_mut().insert(id, d);
        Ok(Class::instance(a.ctx.clone(), Handle { id })?.into_value())
    });
    // sqliteQuery(h, sql, params, mode): 0 = objects, 1 = the first object
    // or null, 2 = arrays
    n.set("sqliteQuery", |h, a| {
        let sql = a.str(1);
        let p = params(a, 2)?;
        let mode = a.int(3);
        let rows = with_db(h, a, |d| d.query(&sql, &p, (mode == 1).then_some(1)))?;
        let names = unique_names(&rows.columns);
        let mut out = Vec::with_capacity(rows.rows.len());
        for r in rows.rows {
            let vals = r.into_iter().enumerate().map(|(i, v)| to_js(&a.ctx, v, rows.decl[i].as_deref())).collect::<rquickjs::Result<Vec<_>>>()?;
            if mode == 2 {
                out.push(js::array(&a.ctx, vals)?);
            } else {
                let o = rquickjs::Object::new(a.ctx.clone())?;
                for (k, v) in names.iter().zip(vals) {
                    o.set(k.as_str(), v)?;
                }
                out.push(o.into_value());
            }
        }
        if mode == 1 {
            return Ok(out.into_iter().next().unwrap_or_else(|| Value::new_null(a.ctx.clone())));
        }
        js::array(&a.ctx, out)
    });
    n.set("sqliteRun", |h, a| {
        let sql = a.str(1);
        let p = params(a, 2)?;
        let (changes, id) = with_db(h, a, |d| d.run(&sql, &p))?;
        let rowid = if id.abs() <= MAX_SAFE { Value::new_number(a.ctx.clone(), id as f64) } else { BigInt::from_i64(a.ctx.clone(), id)?.into_value() };
        js::object(&a.ctx, vec![("changes", js::v(&a.ctx, changes as f64)), ("lastInsertRowid", rowid)])
    });
    n.set("sqliteExec", |h, a| {
        let sql = a.str(1);
        with_db(h, a, |d| d.exec(&sql))?;
        a.undefined()
    });
    // sqliteInsert(h, table, columns, rows (arrays in the order of columns), verb)
    n.set("sqliteInsert", |h, a| {
        let table = a.str(1);
        let cols = a.strings(2);
        let rows = elems(&a.arg(3))
            .iter()
            .map(|r| elems(r).iter().map(|v| to_sql(a, v)).collect::<rquickjs::Result<Vec<_>>>())
            .collect::<rquickjs::Result<Vec<_>>>()?;
        let verb = match a.str(4).as_str() {
            v @ ("INSERT OR REPLACE" | "INSERT OR IGNORE") => v.to_string(),
            _ => "INSERT".to_string(),
        };
        let n = with_db(h, a, |d| d.insert(&table, &cols, rows, &verb))?;
        a.ret(n as f64)
    });
    n.set("sqliteBegin", |h, a| {
        with_db(h, a, |d| d.begin())?;
        a.undefined()
    });
    n.set("sqliteCommit", |h, a| {
        with_db(h, a, |d| d.commit())?;
        a.undefined()
    });
    n.set("sqliteRollback", |h, a| {
        with_db(h, a, |d| d.rollback())?;
        a.undefined()
    });
    n.set("sqliteInfo", |h, a| {
        let (path, shared, readonly, txn, corrupt, closed) =
            with_db(h, a, |d| Ok((d.path.to_string_lossy().into_owned(), d.shared(), d.opts.readonly, d.in_transaction(), d.corrupt, d.closed)))?;
        let c = &a.ctx;
        js::object(
            c,
            vec![
                ("path", js::v(c, path)),
                ("shared", js::v(c, shared)),
                ("readonly", js::v(c, readonly)),
                ("inTransaction", js::v(c, txn)),
                ("corrupt", js::v(c, corrupt)),
                ("closed", js::v(c, closed)),
            ],
        )
    });
    n.set("sqliteCheck", |h, a| {
        let r = with_db(h, a, |d| d.check())?;
        a.ret(r)
    });
    n.set("sqliteBackup", |h, a| {
        let dest = a.str(1);
        with_db(h, a, |d| d.backup(&dest))?;
        a.undefined()
    });
    n.set("sqliteRepairDb", |h, a| {
        let force = a.bool(1);
        let r = with_db(h, a, |d| d.repair(force))?;
        a.ret(report_json(&r))
    });
    n.set("sqliteClose", |h, a| {
        if let Some(id) = handle_id(&a.arg(0))
            && let Some(d) = h.dbs.borrow_mut().get_mut(&id)
        {
            d.close();
        }
        a.undefined()
    });
    // sqliteRepair(path, force, timeoutMs, keep): a database that is not open.
    n.set("sqliteRepair", |_, a| {
        let path = a.str(0);
        let abs = std::path::absolute(&path).unwrap_or_else(|_| path.clone().into());
        let opts = db::Options { timeout: Duration::from_millis(int_opt(a, 2, 30_000).max(0) as u64), keep: int_opt(a, 3, 10).max(0) as usize, ..Default::default() };
        let r = db::repair(&abs, a.bool(1), &opts).map_err(|e| super::throw_path(&a.ctx, &path, e))?;
        a.ret(report_json(&r))
    });
    n.set("sqliteCheckFile", |_, a| {
        let path = a.str(0);
        if !std::path::Path::new(&path).exists() {
            return Err(a.throw(&crate::Error::path("open", path, std::io::Error::from(std::io::ErrorKind::NotFound))));
        }
        let r = match db::check_file(std::path::Path::new(&path)) {
            Ok(v) => v,
            Err(e) => vec![crate::i18n::error(&e)],
        };
        a.ret(r)
    });
    // sqliteBackups(path): JSON [{generation, time, path}], oldest first.
    n.set("sqliteBackups", |_, a| {
        let path = std::path::absolute(a.str(0)).unwrap_or_default();
        let list: Vec<serde_json::Value> = db::share::Sidecar::find(&path)
            .map(|s| s.snapshots())
            .unwrap_or_default()
            .into_iter()
            .map(|s| serde_json::json!({"generation": s.generation, "time": s.time.format("%Y-%m-%d %H:%M:%S").to_string(), "path": s.path.to_string_lossy()}))
            .collect();
        a.ret(serde_json::Value::Array(list).to_string())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_as_text() {
        assert_eq!(date_text((2026, 4, 1, 0, 0, 0, 0)), "2026-04-01");
        assert_eq!(date_text((2026, 4, 1, 9, 30, 0, 0)), "2026-04-01 09:30:00");
        assert_eq!(date_text((2026, 4, 1, 9, 30, 5, 7)), "2026-04-01 09:30:05.007");
        assert!(DATE_RE.is_match("2026-04-01T09:30:00.123+09:00"));
        assert!(!DATE_RE.is_match("09:30"));
        assert_eq!(unique_names(&["id".into(), "id".into(), "id".into()]), ["id", "id_2", "id_3"]);
    }
}
