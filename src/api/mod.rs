//! The JavaScript surface of xljs: the native functions written in Rust and
//! the prelude that builds the public objects (fs, path, csv, workbooks,
//! pickers, ui, console, polyfills) on top of them.

mod app;
mod csv;
mod dayjs;
mod encoding;
mod fs;
mod numfmt;
mod xlsx_calc;
pub(crate) mod gopath;
pub(crate) mod js;
pub mod remote;
mod ui;
mod vba;
mod xlsx;
mod xlsx_style;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, Persistent, Value};

use crate::engine::Out;
use crate::engine::loop_::{Job, Loop};
use crate::engine::{BUNDLE_NAME, Mapper};
use crate::meta::{AssetFs, Meta};
use crate::ui::Ui;
use crate::{Error, i18n};

pub use app::{WindowAssets, page_url};
pub use encoding::{decode, detect_encoding, encode, norm_encoding};
pub use js::A;

pub(crate) type R<'js> = rquickjs::Result<Value<'js>>;

/// The host environment of a script.
pub struct Env {
    pub ui: Arc<dyn Ui>,
    pub args: Vec<String>,
    /// The absolute path of the script (or the packed exe).
    pub script: String,
    pub version: String,
    pub stdout: Out,
    pub stderr: Out,
    /// The default title of dialogs and windows.
    pub title: String,
    /// The script's metadata block; assets its files (served to windows
    /// under ui::ASSET_ORIGIN). debug enables the developer tools.
    pub meta: Meta,
    pub assets: Option<Arc<dyn AssetFs>>,
    pub debug: bool,
}

impl Env {
    /// Closes resources left open by the script (progress indicators,
    /// windows, workbooks). It is safe to call more than once.
    pub fn ui_cleanup(&self, host: &Host) {
        host.cleanup();
    }
}

/// The state of a running script shared by the natives, the event loop and
/// the engine. It lives on the script thread.
pub struct Host {
    pub env: Env,
    pub lp: Loop,
    pub mapper: Option<Mapper>,
    exit: Cell<Option<i32>>,
    /// Called instead of ending the script by exit() (the debugger).
    pub exit_hook: RefCell<Option<Box<dyn Fn(i32)>>>,
    cleanup: RefCell<Vec<Box<dyn FnOnce()>>>,
    next_id: Cell<u64>,
    pub(crate) books: RefCell<HashMap<u64, xlsx::Book>>,
    callbacks: RefCell<HashMap<u64, Persistent<Function<'static>>>>,
    values: RefCell<HashMap<u64, Persistent<Value<'static>>>>,
    natives: RefCell<Option<Persistent<Object<'static>>>>,
    rejections: RefCell<Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>>,
    /// State of the engine's own (the debugger session).
    pub ext: RefCell<Option<Rc<dyn std::any::Any>>>,
}

impl Host {
    pub fn new(env: Env, mapper: Option<Mapper>) -> Host {
        let mut env = env;
        if env.title.is_empty() {
            env.title = std::path::Path::new(&env.script).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        }
        Host {
            env,
            lp: Loop::new(),
            mapper,
            exit: Cell::new(None),
            exit_hook: RefCell::new(None),
            cleanup: RefCell::new(Vec::new()),
            next_id: Cell::new(0),
            books: RefCell::new(HashMap::new()),
            callbacks: RefCell::new(HashMap::new()),
            values: RefCell::new(HashMap::new()),
            natives: RefCell::new(None),
            rejections: RefCell::new(Vec::new()),
            ext: RefCell::new(None),
        }
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.set(self.next_id.get() + 1);
        self.next_id.get()
    }

    /// Whether exit() was called.
    pub fn exited(&self) -> bool {
        self.exit.get().is_some()
    }

    pub fn exit_code(&self) -> i32 {
        self.exit.get().unwrap_or(0)
    }

    /// Ends the script with code (the first call wins).
    pub fn set_exit(&self, code: i32) {
        if self.exit.get().is_none() {
            self.exit.set(Some(code));
        }
    }

    /// Registers f to run when the script ends.
    pub fn on_cleanup(&self, f: impl FnOnce() + 'static) {
        self.cleanup.borrow_mut().push(Box::new(f));
    }

    /// Closes resources left open by the script, last opened first.
    pub fn cleanup(&self) {
        loop {
            let f = self.cleanup.borrow_mut().pop();
            match f {
                Some(f) => f(),
                None => break,
            }
        }
    }

    /// Drops every JavaScript value the host keeps (before the runtime goes
    /// away).
    pub fn clear(&self) {
        self.cleanup();
        self.lp.clear();
        self.callbacks.borrow_mut().clear();
        self.values.borrow_mut().clear();
        self.natives.borrow_mut().take();
        self.rejections.borrow_mut().clear();
        *self.lp.after.borrow_mut() = None;
        self.exit_hook.borrow_mut().take();
        self.ext.borrow_mut().take();
    }

    /// Keeps a function for a later callback (posted from another thread).
    pub fn keep_fn<'js>(&self, ctx: &Ctx<'js>, f: Function<'js>) -> u64 {
        let id = self.next_id();
        self.callbacks.borrow_mut().insert(id, Persistent::save(ctx, f));
        id
    }

    pub fn kept_fn<'js>(&self, ctx: &Ctx<'js>, id: u64) -> Option<Function<'js>> {
        self.callbacks.borrow().get(&id).and_then(|p| p.clone().restore(ctx).ok())
    }

    pub fn drop_fn(&self, id: u64) {
        self.callbacks.borrow_mut().remove(&id);
    }

    /// Keeps any value (promise resolvers and the like).
    pub fn keep<'js>(&self, ctx: &Ctx<'js>, v: Value<'js>) -> u64 {
        let id = self.next_id();
        self.values.borrow_mut().insert(id, Persistent::save(ctx, v));
        id
    }

    pub fn kept<'js>(&self, ctx: &Ctx<'js>, id: u64) -> Option<Value<'js>> {
        self.values.borrow().get(&id).and_then(|p| p.clone().restore(ctx).ok())
    }

    pub fn drop_kept(&self, id: u64) {
        self.values.borrow_mut().remove(&id);
    }

    /// Posts f to the script thread from any thread (see Loop).
    pub fn poster(&self) -> Poster {
        Poster(self.lp.shared.clone())
    }

    pub fn natives<'js>(&self, ctx: &Ctx<'js>) -> Option<Object<'js>> {
        self.natives.borrow().as_ref().and_then(|p| p.clone().restore(ctx).ok())
    }

    pub(crate) fn track_rejection<'js>(&self, ctx: &Ctx<'js>, promise: Value<'js>, reason: Value<'js>, handled: bool) {
        if handled {
            self.forget_rejection(&promise);
        } else {
            let p = Persistent::save(ctx, promise);
            let r = Persistent::save(ctx, reason);
            self.rejections.borrow_mut().push((p, r));
        }
    }

    pub(crate) fn forget_rejection(&self, promise: &Value) {
        let ctx = promise.ctx().clone();
        self.rejections.borrow_mut().retain(|(p, _)| p.clone().restore(&ctx).map(|v| v != *promise).unwrap_or(true));
    }

    pub(crate) fn first_unhandled<'js>(&self, ctx: &Ctx<'js>) -> Option<Value<'js>> {
        self.rejections.borrow().first().and_then(|(_, r)| r.clone().restore(ctx).ok())
    }

    /// Defines setTimeout and friends.
    pub(crate) fn install_loop<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let g = ctx.globals();
        for (name, repeat) in [("setTimeout", false), ("setInterval", true)] {
            let h = Rc::downgrade(self);
            let f = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<i64> {
                let a = A { ctx: ctx.clone(), v: args.0 };
                let Some(func) = a.arg(0).as_function().cloned() else {
                    return Err(js::throw_type(&ctx, &i18n::t("engine.callbackNotFunction", &[])));
                };
                let rest = a.v.iter().skip(2).cloned().collect();
                Ok(h.upgrade().map(|h| h.lp.add_timer(&ctx, func, a.f64(1), repeat, rest)).unwrap_or(0))
            })?;
            f.set_name(name)?;
            g.set(name, f)?;
        }
        let h = Rc::downgrade(self);
        let f = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<i64> {
            let a = A { ctx: ctx.clone(), v: args.0 };
            let Some(func) = a.arg(0).as_function().cloned() else {
                return Err(js::throw_type(&ctx, &i18n::t("engine.callbackNotFunction", &[])));
            };
            let rest = a.v.iter().skip(1).cloned().collect();
            Ok(h.upgrade().map(|h| h.lp.add_timer(&ctx, func, 0.0, false, rest)).unwrap_or(0))
        })?;
        f.set_name("setImmediate")?;
        g.set("setImmediate", f)?;
        for name in ["clearTimeout", "clearInterval", "clearImmediate"] {
            let h = Rc::downgrade(self);
            let f = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                let a = A { ctx, v: args.0 };
                if let Some(h) = h.upgrade() {
                    h.lp.clear_timer(a.int(0));
                }
            })?;
            f.set_name(name)?;
            g.set(name, f)?;
        }
        Ok(())
    }
}

/// Posts callbacks to the script thread; Send, so UI callbacks can use it.
#[derive(Clone)]
pub struct Poster(Arc<crate::engine::loop_::Shared>);

impl Poster {
    pub fn add_pending(&self) {
        self.0.add_pending();
    }
    pub fn post(&self, f: Job) {
        self.0.post(f);
    }
}

/// Registers natives on the natives object.
pub(crate) struct N<'js> {
    pub ctx: Ctx<'js>,
    pub obj: Object<'js>,
    pub host: Rc<Host>,
}

impl<'js> N<'js> {
    pub fn set(&self, name: &str, f: impl Fn(&Host, &A<'js>) -> R<'js> + 'js) {
        let host = self.host.clone();
        let func = Function::new(self.ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| f(&host, &A { ctx, v: args.0 }))
            .expect("cannot create a native function");
        let _ = func.set_name(name);
        self.obj.set(name, func).expect("cannot set a native");
    }

    pub fn val(&self, name: &str, v: impl rquickjs::IntoJs<'js>) {
        self.obj.set(name, v).expect("cannot set a native value");
    }
}

static PRELUDE_RAW: &str = include_str!("prelude.js");

/// The prelude with Day.js put in (see dayjs::bundle).
static PRELUDE: LazyLock<String> = LazyLock::new(|| PRELUDE_RAW.replacen(dayjs::PLACEHOLDER, &dayjs::bundle(), 1));

/// The prelude: a function expression that receives the natives object.
pub fn prelude_source() -> &'static str {
    &PRELUDE
}

/// Defines the xljs globals: the natives and the prelude. It returns the
/// natives object.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Rc<Host>) -> rquickjs::Result<Object<'js>> {
    let obj = install_natives(ctx, host)?;
    let mut opts = rquickjs::context::EvalOptions::default();
    opts.global = true;
    opts.strict = true;
    opts.filename = Some("xljs:prelude".into());
    let f: Function = ctx.eval_with_options(PRELUDE.as_bytes().to_vec(), opts)?;
    f.call::<_, ()>((obj.clone(),))?;
    Ok(obj)
}

/// Defines the natives (without the prelude).
pub(crate) fn install_natives<'js>(ctx: &Ctx<'js>, host: &Rc<Host>) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    *host.natives.borrow_mut() = Some(Persistent::save(ctx, obj.clone()));
    let n = N { ctx: ctx.clone(), obj: obj.clone(), host: host.clone() };
    install_core(&n);
    fs::install(&n);
    encoding::install(&n);
    csv::install(&n);
    xlsx::install(&n);
    xlsx_style::install(&n);
    vba::install(&n);
    ui::install(&n);
    app::install(&n);
    Ok(obj)
}

fn write_out(out: &Out, s: &str) {
    let mut o = out.lock().unwrap();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
}

use std::io::Write as _;

fn install_core(n: &N) {
    n.set("stdout", |h, a| {
        write_out(&h.env.stdout, &a.str(0));
        a.undefined()
    });
    n.set("stderr", |h, a| {
        write_out(&h.env.stderr, &a.str(0));
        a.undefined()
    });
    n.set("exit", |h, a| {
        let code = if a.arg(0).is_null() || a.arg(0).is_undefined() { 0 } else { a.int(0) as i32 };
        if let Some(hook) = &*h.exit_hook.borrow() {
            hook(code);
            return a.undefined();
        }
        h.set_exit(code);
        // Stop the script right here: the error cannot be caught.
        Err(js::throw_uncatchable(&a.ctx))
    });
    n.set("getenv", |_, a| match std::env::var(a.str(0)) {
        Ok(v) => a.ret(v),
        Err(_) => a.null(),
    });
    let env = &n.host.env;
    n.val("args", env.args.clone());
    n.val("script", env.script.clone());
    n.val("version", env.version.clone());
    n.val("platform", platform());
    n.val("uiName", env.ui.name());
    n.val("lang", i18n::lang());
    n.set("t", |_, a| {
        let args: Vec<String> = a.v.iter().skip(1).map(js::to_str).collect();
        a.ret(i18n::t(&a.str(0), &args))
    });
    n.val("title", n.host.env.title.clone());
}

/// The platform name as Go's runtime.GOOS spells it (the prelude checks
/// "windows").
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    }
}

/// Raises err as a JavaScript Error with a localized message.
pub(crate) fn throw(ctx: &Ctx, e: &Error) -> rquickjs::Error {
    js::throw_msg(ctx, &i18n::error(e))
}

/// Raises the error for the file at path, adding the path unless the error
/// already names it.
pub(crate) fn throw_path(ctx: &Ctx, path: &str, e: Error) -> rquickjs::Error {
    if e.has_path() {
        return throw(ctx, &e);
    }
    throw(ctx, &e.context(path))
}

#[allow(dead_code)]
pub(crate) fn bundle_name() -> &'static str {
    BUNDLE_NAME
}
