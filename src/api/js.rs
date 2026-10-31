//! Helpers for values crossing between Rust and QuickJS.

use rquickjs::convert::Coerced;
use rquickjs::{Array, ArrayBuffer, Constructor, Ctx, FromJs, Function, IntoJs, Object, Value};

use super::R;
use crate::Error;

/// The arguments of a native call.
pub struct A<'js> {
    pub ctx: Ctx<'js>,
    pub v: Vec<Value<'js>>,
}

pub fn nullish(v: &Value) -> bool {
    v.is_null() || v.is_undefined()
}

/// String(v) (the JavaScript conversion).
pub fn to_str(v: &Value) -> String {
    if let Some(s) = v.as_string() {
        return s.to_string().unwrap_or_default();
    }
    Coerced::<String>::from_js(v.ctx(), v.clone()).map(|c| c.0).unwrap_or_default()
}

/// Number(v).
pub fn to_f64(v: &Value) -> f64 {
    if let Some(n) = v.as_number() {
        return n;
    }
    if v.is_undefined() {
        return f64::NAN;
    }
    Coerced::<f64>::from_js(v.ctx(), v.clone()).map(|c| c.0).unwrap_or(f64::NAN)
}

/// ToInteger: truncated, NaN and infinities as 0 (like goja's ToInteger).
pub fn to_int(v: &Value) -> i64 {
    let f = to_f64(v);
    if f.is_finite() { f.trunc() as i64 } else { 0 }
}

/// Boolean(v).
pub fn to_bool(v: &Value) -> bool {
    if let Some(b) = v.as_bool() {
        return b;
    }
    Coerced::<bool>::from_js(v.ctx(), v.clone()).map(|c| c.0).unwrap_or(false)
}

impl<'js> A<'js> {
    pub fn arg(&self, i: usize) -> Value<'js> {
        self.v.get(i).cloned().unwrap_or_else(|| Value::new_undefined(self.ctx.clone()))
    }
    /// The argument as a string ("" for null and undefined).
    pub fn str(&self, i: usize) -> String {
        let v = self.arg(i);
        if nullish(&v) { String::new() } else { to_str(&v) }
    }
    pub fn opt_str(&self, i: usize, def: &str) -> String {
        let v = self.arg(i);
        if nullish(&v) { def.to_string() } else { to_str(&v) }
    }
    pub fn f64(&self, i: usize) -> f64 {
        to_f64(&self.arg(i))
    }
    pub fn int(&self, i: usize) -> i64 {
        to_int(&self.arg(i))
    }
    pub fn bool(&self, i: usize) -> bool {
        to_bool(&self.arg(i))
    }
    pub fn strings(&self, i: usize) -> Vec<String> {
        strings(&self.arg(i))
    }
    pub fn bytes(&self, i: usize) -> rquickjs::Result<Vec<u8>> {
        bytes(&self.ctx, &self.arg(i))
    }
    pub fn undefined(&self) -> R<'js> {
        Ok(Value::new_undefined(self.ctx.clone()))
    }
    pub fn null(&self) -> R<'js> {
        Ok(Value::new_null(self.ctx.clone()))
    }
    pub fn ret(&self, v: impl IntoJs<'js>) -> R<'js> {
        v.into_js(&self.ctx)
    }
    pub fn throw(&self, e: &Error) -> rquickjs::Error {
        super::throw(&self.ctx, e)
    }
    pub fn throw_t(&self, key: &str, args: &[String]) -> rquickjs::Error {
        throw_msg(&self.ctx, &crate::i18n::t(key, args))
    }
}

/// The elements of an array-like value.
pub fn elems<'js>(v: &Value<'js>) -> Vec<Value<'js>> {
    if nullish(v) {
        return Vec::new();
    }
    if let Some(a) = v.as_array() {
        return a.iter::<Value>().filter_map(Result::ok).collect();
    }
    let Some(o) = v.as_object() else { return Vec::new() };
    let n = o.get::<_, Value>("length").map(|l| to_int(&l)).unwrap_or(0).max(0);
    (0..n).map(|i| o.get::<_, Value>(i as u32).unwrap_or_else(|_| Value::new_undefined(v.ctx().clone()))).collect()
}

/// A string or an array of strings (nullish elements are skipped).
pub fn strings(v: &Value) -> Vec<String> {
    if nullish(v) {
        return Vec::new();
    }
    if v.is_string() {
        return vec![to_str(v)];
    }
    elems(v).iter().filter(|e| !nullish(e)).map(to_str).collect()
}

/// An ArrayBuffer, typed array, DataView or string as bytes.
pub fn bytes<'js>(ctx: &Ctx<'js>, v: &Value<'js>) -> rquickjs::Result<Vec<u8>> {
    if nullish(v) {
        return Ok(Vec::new());
    }
    if v.is_string() {
        return Ok(to_str(v).into_bytes());
    }
    if let Some(o) = v.as_object() {
        if let Some(ab) = ArrayBuffer::from_object(o.clone())
            && let Some(b) = unsafe { ab.as_bytes() }
        {
            return Ok(b.to_vec());
        }
        if let Ok(buf) = o.get::<_, Value>("buffer")
            && let Some(bo) = buf.as_object()
            && let Some(ab) = ArrayBuffer::from_object(bo.clone())
            && let Some(b) = unsafe { ab.as_bytes() }
        {
            let off = o.get::<_, Value>("byteOffset").map(|v| to_int(&v)).unwrap_or(0).max(0) as usize;
            let len = o.get::<_, Value>("byteLength").map(|v| to_int(&v)).unwrap_or(0).max(0) as usize;
            if off + len <= b.len() {
                return Ok(b[off..off + len].to_vec());
            }
        }
    }
    Err(throw_msg(ctx, &crate::i18n::t("data.expectedBytes", &[])))
}

pub fn array_buffer<'js>(ctx: &Ctx<'js>, data: Vec<u8>) -> R<'js> {
    Ok(ArrayBuffer::new(ctx.clone(), data)?.into_value())
}

/// Builds an array of values.
pub fn array<'js>(ctx: &Ctx<'js>, vals: impl IntoIterator<Item = Value<'js>>) -> R<'js> {
    let a = Array::new(ctx.clone())?;
    for (i, v) in vals.into_iter().enumerate() {
        a.set(i, v)?;
    }
    Ok(a.into_value())
}

/// A plain object from key/value pairs, in order.
pub fn object<'js>(ctx: &Ctx<'js>, pairs: Vec<(&str, Value<'js>)>) -> R<'js> {
    let o = Object::new(ctx.clone())?;
    for (k, v) in pairs {
        o.set(k, v)?;
    }
    Ok(o.into_value())
}

pub fn v<'js, T: IntoJs<'js>>(ctx: &Ctx<'js>, x: T) -> Value<'js> {
    x.into_js(ctx).unwrap_or_else(|_| Value::new_undefined(ctx.clone()))
}

fn construct<'js>(ctx: &Ctx<'js>, name: &str, msg: &str) -> Value<'js> {
    let c: Option<Constructor> = ctx.globals().get(name).ok();
    c.and_then(|c| c.construct::<_, Value>((msg,)).ok()).unwrap_or_else(|| v(ctx, msg))
}

/// Raises an Error with the message.
pub fn throw_msg(ctx: &Ctx, msg: &str) -> rquickjs::Error {
    ctx.throw(construct(ctx, "Error", msg))
}

/// Raises a TypeError with the message.
pub fn throw_type(ctx: &Ctx, msg: &str) -> rquickjs::Error {
    ctx.throw(construct(ctx, "TypeError", msg))
}

/// Raises an error no try/catch can stop (exit()).
pub fn throw_uncatchable(ctx: &Ctx) -> rquickjs::Error {
    let e = construct(ctx, "Error", "exit");
    unsafe { rquickjs::qjs::JS_SetUncatchableError(ctx.as_raw().as_ptr(), e.as_raw()) };
    ctx.throw(e)
}

/// new Date(ms).
pub fn date_ms<'js>(ctx: &Ctx<'js>, ms: f64) -> R<'js> {
    let c: Constructor = ctx.globals().get("Date")?;
    c.construct((ms,))
}

/// A Date whose local wall-clock time is the given one.
pub fn date_local<'js>(ctx: &Ctx<'js>, y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32) -> R<'js> {
    let c: Constructor = ctx.globals().get("Date")?;
    let args = rquickjs::function::Args::new(ctx.clone(), 7);
    let mut args = args;
    for x in [y as f64, mo as f64 - 1.0, d as f64, h as f64, mi as f64, s as f64, ms as f64] {
        args.push_arg(x)?;
    }
    let d: Value = c.construct_args(args)?;
    // Years 0..99 mean 1900.. for the constructor; set them again.
    if (0..100).contains(&y)
        && let Some(o) = d.as_object()
    {
        let f: Function = o.get("setFullYear")?;
        f.call::<_, Value>((rquickjs::function::This(o.clone()), y as f64))?;
    }
    Ok(d)
}

/// Whether v is a Date.
pub fn is_date(v: &Value) -> bool {
    let Some(o) = v.as_object() else { return false };
    let Ok(c) = v.ctx().globals().get::<_, Constructor>("Date") else { return false };
    o.is_instance_of(&c)
}

/// The local wall-clock fields of a Date: (y, month 1-12, d, h, mi, s, ms),
/// None for an invalid date.
pub fn date_fields(v: &Value) -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
    let o = v.as_object()?;
    let call = |name: &str| -> Option<f64> {
        let f: Function = o.get(name).ok()?;
        f.call::<_, f64>((rquickjs::function::This(o.clone()),)).ok()
    };
    let y = call("getFullYear")?;
    if y.is_nan() {
        return None;
    }
    Some((
        y as i32,
        call("getMonth")? as u32 + 1,
        call("getDate")? as u32,
        call("getHours")? as u32,
        call("getMinutes")? as u32,
        call("getSeconds")? as u32,
        call("getMilliseconds")? as u32,
    ))
}

/// The time value of a Date (NaN when invalid).
pub fn date_value(v: &Value) -> f64 {
    let Some(o) = v.as_object() else { return f64::NAN };
    let f: Option<Function> = o.get("getTime").ok();
    f.and_then(|f| f.call::<_, f64>((rquickjs::function::This(o.clone()),)).ok()).unwrap_or(f64::NAN)
}

/// A property of an object (undefined when missing).
pub fn get<'js>(o: &Value<'js>, key: &str) -> Value<'js> {
    o.as_object().and_then(|o| o.get::<_, Value>(key).ok()).unwrap_or_else(|| Value::new_undefined(o.ctx().clone()))
}

/// Whether the object has the key with a value other than undefined.
pub fn has(o: &Value, key: &str) -> bool {
    !get(o, key).is_undefined()
}
