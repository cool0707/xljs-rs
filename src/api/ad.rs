//! Natives of the ad object (Active Directory, see crate::ad).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{Ctx, Value};

use super::js::{self, to_str};
use super::{N, R};
use crate::ad::{Cond, Obj, Scope, SearchOptions, Session, Val};
use crate::{Result, err};

fn val<'js>(ctx: &Ctx<'js>, v: Val) -> R<'js> {
    Ok(match v {
        Val::Null => Value::new_null(ctx.clone()),
        Val::Bool(b) => js::v(ctx, b),
        Val::Num(n) => js::v(ctx, n),
        Val::Str(s) => js::v(ctx, s),
        Val::Date(ms) => js::date_ms(ctx, ms)?,
        Val::Bytes(b) => js::array_buffer(ctx, b)?,
        Val::List(l) => js::array(ctx, l.into_iter().map(|x| val(ctx, x)).collect::<rquickjs::Result<Vec<_>>>()?)?,
    })
}

fn obj<'js>(ctx: &Ctx<'js>, o: Obj) -> R<'js> {
    let out = rquickjs::Object::new(ctx.clone())?;
    for (k, v) in o {
        out.set(k, val(ctx, v)?)?;
    }
    Ok(out.into_value())
}

fn keys(o: &Value) -> Vec<String> {
    o.as_object().map(|o| o.keys::<String>().flatten().collect()).unwrap_or_default()
}

fn opt_bool(v: &Value) -> Option<bool> {
    if js::nullish(v) { None } else { Some(js::to_bool(v)) }
}

fn opt_num(v: &Value) -> Option<f64> {
    if js::nullish(v) { None } else { Some(js::to_f64(v)).filter(|n| n.is_finite()) }
}

fn opt_str(v: &Value) -> Option<String> {
    if js::nullish(v) { None } else { Some(to_str(v)) }
}

/// A value of the filter object: a string (with "*"), true / false
/// (present or not), null (not present), or an array (any of them).
fn cond(v: &Value) -> Cond {
    if v.is_null() || v.is_undefined() {
        return Cond::Present(false);
    }
    if let Some(b) = v.as_bool() {
        return Cond::Present(b);
    }
    if v.is_array() {
        return Cond::Any(js::elems(v).iter().map(cond).collect());
    }
    Cond::Eq(to_str(v))
}

/// Checks that the options object has only the given keys (undefined
/// values aside).
fn check_keys(o: &Value, allowed: &[&str], func: &str) -> Result<()> {
    for k in keys(o) {
        if !allowed.contains(&k.as_str()) && !js::get(o, &k).is_undefined() {
            return Err(err!("ad.unknownOption", k, func, allowed.join(", ")));
        }
    }
    Ok(())
}

const SEARCH_KEYS: &[&str] = &[
    "type", "name", "filter", "disabled", "lockedOut", "expired", "expiringDays", "inactiveDays", "passwordExpired",
    "passwordNeverExpires", "memberOf", "recursive", "searchBase", "scope", "properties", "limit", "server",
];

fn search_options(o: &Value) -> Result<(SearchOptions, String)> {
    check_keys(o, SEARCH_KEYS, "ad.search")?;
    let g = |k: &str| js::get(o, k);
    let mut s = SearchOptions {
        types: js::strings(&g("type")),
        name: opt_str(&g("name")).filter(|n| !n.is_empty()),
        disabled: opt_bool(&g("disabled")),
        locked_out: opt_bool(&g("lockedOut")),
        expired: opt_bool(&g("expired")),
        expiring_days: opt_num(&g("expiringDays")),
        inactive_days: opt_num(&g("inactiveDays")),
        password_expired: opt_bool(&g("passwordExpired")),
        password_never_expires: opt_bool(&g("passwordNeverExpires")),
        member_of: opt_str(&g("memberOf")).filter(|n| !n.is_empty()),
        recursive: js::to_bool(&g("recursive")),
        base: opt_str(&g("searchBase")).unwrap_or_default(),
        properties: js::strings(&g("properties")),
        limit: js::to_int(&g("limit")).max(0) as usize,
        ..Default::default()
    };
    s.scope = match opt_str(&g("scope")).unwrap_or_default().to_ascii_lowercase().as_str() {
        "" | "subtree" => Scope::Subtree,
        "onelevel" => Scope::OneLevel,
        "base" => Scope::Base,
        other => return Err(err!("ad.badScope", other)),
    };
    let f = g("filter");
    for f in if f.is_array() { js::elems(&f) } else { vec![f] } {
        if js::nullish(&f) {
            continue;
        }
        if f.is_string() {
            s.filters.push(to_str(&f));
        } else if f.is_object() {
            for k in keys(&f) {
                let v = js::get(&f, &k);
                // { department: me.department } of someone without one would
                // otherwise list everyone without a department.
                if v.is_undefined() {
                    return Err(err!("ad.undefinedFilter", k));
                }
                let c = cond(&v);
                s.conds.push((k, c));
            }
        }
    }
    Ok((s, opt_str(&g("server")).unwrap_or_default()))
}

/// Open connections by server ("" for the computer's domain).
type Sessions = Rc<RefCell<HashMap<String, Rc<Session>>>>;

fn session(cache: &Sessions, server: &str) -> Result<Rc<Session>> {
    if let Some(s) = cache.borrow().get(server) {
        return Ok(s.clone());
    }
    let s = Rc::new(Session::open(server)?);
    cache.borrow_mut().insert(server.to_string(), s.clone());
    Ok(s)
}

/// Runs f against the directory; the time it took does not count for the
/// watchdog.
fn with<T>(cache: &Sessions, server: &str, f: impl FnOnce(&Session) -> Result<T>) -> Result<T> {
    let r = session(cache, server).and_then(|s| f(&s));
    crate::engine::watchdog::waited();
    r
}

pub(super) fn install(n: &N) {
    let cache: Sessions = Rc::default();
    let c = cache.clone();
    n.host.on_cleanup(move || c.borrow_mut().clear());

    n.set("adJoined", |_, a| a.ret(crate::ad::joined()));
    let c = cache.clone();
    // adDomain(server) -> { name, netbiosName, distinguishedName, forest, server }
    n.set("adDomain", move |_, a| {
        let d = with(&c, &a.str(0), |s| s.domain()).map_err(|e| a.throw(&e))?;
        obj(&a.ctx, d)
    });
    let c = cache.clone();
    // adMe(computer, { properties, server }) -> object
    n.set("adMe", move |_, a| {
        let o = a.arg(1);
        let func = if a.bool(0) { "ad.computer" } else { "ad.me" };
        check_keys(&o, &["properties", "server"], func).map_err(|e| a.throw(&e))?;
        let props = js::strings(&js::get(&o, "properties"));
        let server = opt_str(&js::get(&o, "server")).unwrap_or_default();
        let me = with(&c, &server, |s| s.me(a.bool(0), &props)).map_err(|e| a.throw(&e))?;
        obj(&a.ctx, me)
    });
    let c = cache.clone();
    // adGet(identity, { type, properties, server }) -> object | null
    n.set("adGet", move |_, a| {
        let o = a.arg(1);
        check_keys(&o, &["type", "properties", "server"], "ad.get").map_err(|e| a.throw(&e))?;
        let types = js::strings(&js::get(&o, "type"));
        let props = js::strings(&js::get(&o, "properties"));
        let server = opt_str(&js::get(&o, "server")).unwrap_or_default();
        match with(&c, &server, |s| s.get(&a.str(0), &types, &props)).map_err(|e| a.throw(&e))? {
            Some(x) => obj(&a.ctx, x),
            None => a.null(),
        }
    });
    let c = cache;
    // adSearch(options) -> object[]
    n.set("adSearch", move |_, a| {
        let (o, server) = search_options(&a.arg(0)).map_err(|e| a.throw(&e))?;
        let found = with(&c, &server, |s| s.search(&o)).map_err(|e| a.throw(&e))?;
        js::array(&a.ctx, found.into_iter().map(|x| obj(&a.ctx, x)).collect::<rquickjs::Result<Vec<_>>>()?)
    });
}
