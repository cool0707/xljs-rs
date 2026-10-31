//! Serves the natives to a script that runs in another JavaScript engine:
//! the debugger page (ui/web/debug.js), where the prelude and the script
//! run in WebView2 so that its developer tools can debug them. Calls arrive
//! as JSON; values travel in this wire format:
//!
//! ```text
//! null, booleans, numbers, strings, arrays  as in JSON
//! {"$o": {...}}                             plain object (keys in order)
//! {"$u": 0}                                 undefined
//! {"$num": "NaN"}                           NaN, Infinity, -Infinity, -0
//! {"$big": "123"}                           bigint
//! {"$date": ms}                             Date (null = invalid date)
//! {"$bytes": base64, "$t": "Uint8Array"}    ArrayBuffer or typed array
//! {"$err": {"name", "message", "stack"}}    Error
//! {"$fn": id}                               function of the page
//! {"$ref": id, "$k": "fn"|"obj"}            function or object of this side
//! {"$p": id}                                promise of this side
//! ```
//!
//! Functions of the page are called with an event {"ev": "call", "fn",
//! "args"} (without waiting for them); promises of this side are settled
//! with {"ev": "settle", "p", "ok", "value"}. Objects of this side
//! (workbooks, windows) stay here and are referred to by id. A Remote must
//! only be used on the script thread.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use base64::Engine as _;
use rquickjs::function::{Rest, This};
use rquickjs::{Array, ArrayBuffer, Constructor, Ctx, Function, Object, Persistent, Value};
use serde_json::{Map, Value as J, json};

use super::js::{self, to_str};
use super::{Host, install_natives};

const TYPED_ARRAYS: &[&str] = &[
    "Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array", "Float32Array",
    "Float64Array", "BigInt64Array", "BigUint64Array", "DataView",
];

struct Inner {
    natives: Persistent<Object<'static>>,
    refs: RefCell<Vec<(i64, Persistent<Value<'static>>)>>,
    next: Cell<i64>,
    sent: Cell<usize>,
    send: RefCell<Option<Box<dyn Fn(String)>>>,
}

/// The natives behind the wire format (see the module documentation).
#[derive(Clone)]
pub struct Remote(Rc<Inner>);

fn plain_error(name: &str, message: &str, stack: &str) -> J {
    json!({"$err": {"name": name, "message": message, "stack": stack}})
}

impl Remote {
    /// Defines the natives in the context (without the prelude, which runs
    /// in the page).
    pub fn new<'js>(ctx: &Ctx<'js>, host: &Rc<Host>) -> rquickjs::Result<Remote> {
        let obj = install_natives(ctx, host)?;
        Ok(Remote(Rc::new(Inner {
            natives: Persistent::save(ctx, obj),
            refs: RefCell::new(Vec::new()),
            next: Cell::new(0),
            sent: Cell::new(0),
            send: RefCell::new(None),
        })))
    }

    /// Sets where events to the page go.
    pub fn set_send(&self, f: impl Fn(String) + 'static) {
        *self.0.send.borrow_mut() = Some(Box::new(f));
    }

    /// The number of events sent to the page so far.
    pub fn sent(&self) -> usize {
        self.0.sent.get()
    }

    /// Drops the values kept for the page.
    pub fn clear(&self) {
        self.0.refs.borrow_mut().clear();
        self.0.send.borrow_mut().take();
    }

    fn send(&self, ev: J) {
        self.0.sent.set(self.0.sent.get() + 1);
        if let Some(f) = &*self.0.send.borrow() {
            f(ev.to_string());
        }
    }

    /// The natives object in the wire format: its functions become
    /// references the page can call.
    pub fn manifest<'js>(&self, ctx: &Ctx<'js>) -> J {
        match self.0.natives.clone().restore(ctx) {
            Ok(o) => self.encode(ctx, &o.into_value(), 0),
            Err(_) => J::Null,
        }
    }

    /// Calls the native name; the response to the page is {"value": v} or
    /// {"error": e}.
    pub fn call<'js>(&self, ctx: &Ctx<'js>, name: &str, args: &[J]) -> J {
        let f = self.0.natives.clone().restore(ctx).ok().and_then(|o| o.get::<_, Function>(name).ok());
        match f {
            Some(f) => self.call_fn(ctx, f, args),
            None => json!({"error": plain_error("Error", &format!("unknown native {name:?}"), "")}),
        }
    }

    /// Calls the function with reference id.
    pub fn invoke<'js>(&self, ctx: &Ctx<'js>, id: i64, args: &[J]) -> J {
        match self.get_ref(ctx, id).and_then(|v| v.as_function().cloned()) {
            Some(f) => self.call_fn(ctx, f, args),
            None => json!({"error": plain_error("Error", &format!("unknown function reference {id}"), "")}),
        }
    }

    fn call_fn<'js>(&self, ctx: &Ctx<'js>, f: Function<'js>, args: &[J]) -> J {
        let mut a = rquickjs::function::Args::new(ctx.clone(), args.len());
        for x in args {
            match self.decode(ctx, x) {
                Ok(v) => {
                    let _ = a.push_arg(v);
                }
                Err(e) => return json!({"error": plain_error("Error", &e, "")}),
            }
        }
        match f.call_arg::<Value>(a) {
            Ok(v) => json!({"value": self.encode(ctx, &v, 0)}),
            Err(rquickjs::Error::Exception) => {
                let ex = ctx.catch();
                json!({"error": self.encode(ctx, &ex, 0)})
            }
            Err(e) => json!({"error": plain_error("Error", &e.to_string(), "")}),
        }
    }

    fn get_ref<'js>(&self, ctx: &Ctx<'js>, id: i64) -> Option<Value<'js>> {
        self.0.refs.borrow().iter().find(|(i, _)| *i == id).and_then(|(_, p)| p.clone().restore(ctx).ok())
    }

    /// The id of a value of this side, registering it (new: true).
    fn reference<'js>(&self, ctx: &Ctx<'js>, v: &Value<'js>) -> (i64, bool) {
        for (id, p) in self.0.refs.borrow().iter() {
            if let Ok(x) = p.clone().restore(ctx)
                && x == *v
            {
                return (*id, false);
            }
        }
        self.0.next.set(self.0.next.get() + 1);
        let id = self.0.next.get();
        self.0.refs.borrow_mut().push((id, Persistent::save(ctx, v.clone())));
        (id, true)
    }

    fn encode<'js>(&self, ctx: &Ctx<'js>, v: &Value<'js>, depth: usize) -> J {
        if depth > 100 {
            return plain_error("TypeError", "value is nested too deeply (circular?)", "");
        }
        if v.is_undefined() {
            return json!({"$u": 0});
        }
        if v.is_null() {
            return J::Null;
        }
        if let Some(b) = v.as_bool() {
            return J::Bool(b);
        }
        if let Some(i) = v.as_int() {
            return json!(i);
        }
        if let Some(f) = v.as_float() {
            return number(f);
        }
        if v.is_string() {
            return J::String(to_str(v));
        }
        if v.is_big_int() {
            return json!({"$big": to_str(v)});
        }
        if v.is_symbol() {
            return json!({"$u": 0});
        }
        if v.is_function() {
            let (id, _) = self.reference(ctx, v);
            return json!({"$ref": id, "$k": "fn"});
        }
        if v.is_promise() {
            let (id, fresh) = self.reference(ctx, v);
            if fresh {
                self.watch(ctx, v, id);
            }
            return json!({"$p": id});
        }
        let Some(o) = v.as_object() else { return json!({"$u": 0}) };
        if let Some(a) = v.as_array() {
            return J::Array(a.iter::<Value>().map(|x| x.map(|x| self.encode(ctx, &x, depth + 1)).unwrap_or(J::Null)).collect());
        }
        if js::is_date(v) {
            let t = date_time(v);
            return if t.is_nan() { json!({"$date": null}) } else { json!({"$date": t}) };
        }
        if is_instance(ctx, o, "Error") {
            let s = |k: &str| {
                let x = js::get(v, k);
                if js::nullish(&x) { String::new() } else { to_str(&x) }
            };
            return plain_error(&s("name"), &s("message"), &s("stack"));
        }
        if let Some(ab) = ArrayBuffer::from_object(o.clone()) {
            let data = unsafe { ab.as_bytes() }.map(<[u8]>::to_vec).unwrap_or_default();
            return json!({"$bytes": base64::engine::general_purpose::STANDARD.encode(data), "$t": "ArrayBuffer"});
        }
        if let Some(t) = typed_array_name(o) {
            let data = js::bytes(ctx, v).unwrap_or_default();
            return json!({"$bytes": base64::engine::general_purpose::STANDARD.encode(data), "$t": t});
        }
        if is_plain(ctx, o) {
            let mut m = Map::new();
            for k in o.keys::<String>().flatten() {
                let x: Value = o.get(&k).unwrap_or_else(|_| Value::new_undefined(ctx.clone()));
                m.insert(k, self.encode(ctx, &x, depth + 1));
            }
            return json!({"$o": m});
        }
        let (id, _) = self.reference(ctx, v);
        json!({"$ref": id, "$k": "obj"})
    }

    /// Tells the page when the promise settles.
    fn watch<'js>(&self, ctx: &Ctx<'js>, p: &Value<'js>, id: i64) {
        let Some(o) = p.as_object() else { return };
        let Ok(then) = o.get::<_, Function>("then") else { return };
        let settle = |ok: bool| {
            let r = Rc::downgrade(&self.0);
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                if let Some(inner) = r.upgrade() {
                    let r = Remote(inner);
                    let v = args.0.first().cloned().unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                    let value = r.encode(&ctx, &v, 0);
                    r.send(json!({"ev": "settle", "p": id, "ok": ok, "value": value}));
                }
            })
        };
        if let (Ok(a), Ok(b)) = (settle(true), settle(false)) {
            let _ = then.call::<_, Value>((This(o.clone()), a, b));
        }
    }

    /// Converts a value in the wire format.
    pub fn decode<'js>(&self, ctx: &Ctx<'js>, x: &J) -> Result<Value<'js>, String> {
        let e = |r: rquickjs::Result<Value<'js>>| r.map_err(|e| e.to_string());
        match x {
            J::Null => Ok(Value::new_null(ctx.clone())),
            J::Bool(b) => Ok(Value::new_bool(ctx.clone(), *b)),
            J::Number(n) => Ok(match n.as_i64() {
                Some(i) if i32::try_from(i).is_ok() => Value::new_int(ctx.clone(), i as i32),
                _ => Value::new_number(ctx.clone(), n.as_f64().unwrap_or(f64::NAN)),
            }),
            J::String(s) => e(rquickjs::IntoJs::into_js(s.as_str(), ctx)),
            J::Array(a) => {
                let arr = Array::new(ctx.clone()).map_err(|e| e.to_string())?;
                for (i, x) in a.iter().enumerate() {
                    arr.set(i, self.decode(ctx, x)?).map_err(|e| e.to_string())?;
                }
                Ok(arr.into_value())
            }
            J::Object(m) => self.decode_tagged(ctx, m),
        }
    }

    fn decode_tagged<'js>(&self, ctx: &Ctx<'js>, m: &Map<String, J>) -> Result<Value<'js>, String> {
        let es = |e: rquickjs::Error| e.to_string();
        if let Some(J::Object(o)) = m.get("$o") {
            let obj = Object::new(ctx.clone()).map_err(es)?;
            for (k, x) in o {
                obj.set(k.as_str(), self.decode(ctx, x)?).map_err(es)?;
            }
            return Ok(obj.into_value());
        }
        let s = |k: &str| m.get(k).and_then(J::as_str).unwrap_or("").to_string();
        let n = |k: &str| m.get(k).and_then(J::as_i64).unwrap_or(0);
        if m.contains_key("$u") {
            return Ok(Value::new_undefined(ctx.clone()));
        }
        if m.contains_key("$num") {
            let f = match s("$num").as_str() {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                "-0" => -0.0,
                x => x.parse().map_err(|_| format!("bad number {x}"))?,
            };
            return Ok(Value::new_number(ctx.clone(), f));
        }
        if m.contains_key("$big") {
            let f: Function = ctx.globals().get("BigInt").map_err(es)?;
            return f.call((s("$big"),)).map_err(es);
        }
        if let Some(d) = m.get("$date") {
            let ms = d.as_f64().unwrap_or(f64::NAN);
            return js::date_ms(ctx, ms).map_err(es);
        }
        if m.contains_key("$bytes") {
            let data = base64::engine::general_purpose::STANDARD.decode(s("$bytes")).map_err(|e| e.to_string())?;
            let buf = ArrayBuffer::new(ctx.clone(), data).map_err(es)?.into_value();
            let t = s("$t");
            if t == "ArrayBuffer" {
                return Ok(buf);
            }
            if !TYPED_ARRAYS.contains(&t.as_str()) {
                return Err(format!("unknown typed array {t:?}"));
            }
            let c: Constructor = ctx.globals().get(t.as_str()).map_err(es)?;
            return c.construct((buf,)).map_err(es);
        }
        if let Some(J::Object(err)) = m.get("$err") {
            let g = |k: &str| err.get(k).and_then(J::as_str).unwrap_or("").to_string();
            let name = g("name");
            let ctor = if ["Error", "TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError", "URIError"].contains(&name.as_str()) {
                name.clone()
            } else {
                "Error".into()
            };
            let c: Constructor = ctx.globals().get(ctor.as_str()).map_err(es)?;
            let o: Object = c.construct((g("message"),)).map_err(es)?;
            if name != ctor {
                let _ = o.set("name", name);
            }
            let stack = g("stack");
            if !stack.is_empty() {
                let _ = o.prop("stack", rquickjs::object::Property::from(stack).writable().configurable());
            }
            return Ok(o.into_value());
        }
        if m.contains_key("$fn") {
            let id = n("$fn");
            let r = Rc::downgrade(&self.0);
            let f = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                if let Some(inner) = r.upgrade() {
                    let r = Remote(inner);
                    let args: Vec<J> = args.0.iter().map(|a| r.encode(&ctx, a, 0)).collect();
                    r.send(json!({"ev": "call", "fn": id, "args": args}));
                }
            })
            .map_err(es)?;
            return Ok(f.into_value());
        }
        if m.contains_key("$ref") || m.contains_key("$p") {
            let id = if m.contains_key("$p") { n("$p") } else { n("$ref") };
            return self.get_ref(ctx, id).ok_or_else(|| format!("unknown reference {id}"));
        }
        Err("untagged object in the wire format".into())
    }
}

fn number(f: f64) -> J {
    if f.is_nan() {
        json!({"$num": "NaN"})
    } else if f.is_infinite() {
        json!({"$num": if f > 0.0 { "Infinity" } else { "-Infinity" }})
    } else if f == 0.0 && f.is_sign_negative() {
        json!({"$num": "-0"})
    } else if f == f.trunc() && f.abs() < 9.0e15 {
        json!(f as i64)
    } else {
        json!(f)
    }
}

fn date_time(v: &Value) -> f64 {
    let Some(o) = v.as_object() else { return f64::NAN };
    o.get::<_, Function>("getTime").and_then(|f| f.call::<_, f64>((This(o.clone()),))).unwrap_or(f64::NAN)
}

fn is_instance<'js>(ctx: &Ctx<'js>, o: &Object<'js>, ctor: &str) -> bool {
    ctx.globals().get::<_, Constructor>(ctor).map(|c| o.is_instance_of(&c)).unwrap_or(false)
}

/// The constructor name of a typed array or DataView.
fn typed_array_name(o: &Object) -> Option<String> {
    let c: Object = o.get("constructor").ok()?;
    let name: String = c.get("name").ok()?;
    if TYPED_ARRAYS.contains(&name.as_str()) && o.get::<_, Object>("buffer").ok().and_then(ArrayBuffer::from_object).is_some() {
        return Some(name);
    }
    None
}

/// Whether the object is a plain object (its prototype is Object.prototype
/// or null).
fn is_plain<'js>(ctx: &Ctx<'js>, o: &Object<'js>) -> bool {
    match o.get_prototype() {
        None => true,
        Some(p) => {
            let op: Option<Object> = ctx.globals().get::<_, Object>("Object").ok().and_then(|c| c.get("prototype").ok());
            op.is_some_and(|op| op == p)
        }
    }
}
