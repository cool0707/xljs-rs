//! Runs xljs scripts: bundles the entry file, sets up the QuickJS runtime
//! with the xljs API and drives the event loop.

mod bundle;
mod debug;
pub(crate) mod loop_;
mod sourcemap;

use std::fmt;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use rquickjs::{CatchResultExt, Ctx, Function, Object, Value};

use crate::api::{self, Env, Host};
use crate::meta::{AssetFs, Meta};
use crate::ui::Ui;
use crate::{Error, Result, err, i18n, t};

pub use bundle::{EXPORTS_PARAM, bundle, bundle_with};
pub use debug::DEBUG_PAGE;
pub use sourcemap::Mapper;

/// Where the script's output goes.
pub type Out = Arc<Mutex<dyn Write + Send>>;

/// An Out for the process stdout / stderr.
pub fn stdout() -> Out {
    Arc::new(Mutex::new(std::io::stdout()))
}
pub fn stderr() -> Out {
    Arc::new(Mutex::new(std::io::stderr()))
}

/// The name of the bundle in stack traces before they are mapped.
pub const BUNDLE_NAME: &str = "xljs:bundle";

/// Configures a script run.
#[derive(Clone)]
#[derive(Default)]
pub struct Options {
    /// The entry file. When code is set it is only used for naming and to
    /// derive the script directory.
    pub script_path: String,
    /// An already bundled script (from bundle or a packed exe).
    pub code: Option<Vec<u8>>,
    /// The arguments passed to the script (xljs.args).
    pub args: Vec<String>,
    pub ui: Option<Arc<dyn Ui>>,
    pub stdout: Option<Out>,
    pub stderr: Option<Out>,
    pub version: String,
    /// The script's metadata block. When None it is read from script_path
    /// (warnings go to stderr).
    pub meta: Option<Meta>,
    /// The files served to windows. When None they are read from the script
    /// folder (limited to Meta::asset_roots).
    pub assets: Option<Arc<dyn AssetFs>>,
    /// Enables the developer tools in windows. debug_script runs the script
    /// in the debugger window instead of QuickJS (see debug.rs).
    pub debug: bool,
    pub debug_script: bool,
    /// (tests) Runs the script in the debugger window without waiting for
    /// the user and closes the window at the end.
    pub debug_auto_run: bool,
    /// (tests) Receives what happens between Rust and the debugger page.
    pub debug_trace: Option<Arc<dyn Fn(String) + Send + Sync>>,
}


/// A JavaScript error that terminated the script.
#[derive(Debug, Clone)]
pub struct ScriptError {
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn script_error(message: String) -> Error {
    Error::Script(ScriptError { message })
}

/// One message of a BundleError: (file, line, column, line text) and text.
#[derive(Debug, Clone)]
pub struct BundleMessage {
    pub location: Option<(String, usize, usize, String)>,
    pub text: String,
}

/// Syntax and resolution errors found while bundling.
#[derive(Debug, Clone)]
pub struct BundleError {
    pub messages: Vec<BundleMessage>,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, m) in self.messages.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            match &m.location {
                Some((file, line, col, text)) => {
                    write!(f, "{file}:{line}:{}: {}", col + 1, i18n::text(&m.text))?;
                    if !text.is_empty() {
                        write!(f, "\n    {text}")?;
                    }
                }
                None => f.write_str(&i18n::text(&m.text))?,
            }
        }
        Ok(())
    }
}

pub(crate) fn abs_path(p: &str) -> String {
    std::path::absolute(p).map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string())
}

pub(crate) fn script_dir(script: &str) -> std::path::PathBuf {
    std::path::Path::new(script).parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

/// Fills in the defaults of o (UI, output, metadata, code).
fn prepare(o: &mut Options) -> Result<Vec<u8>> {
    if o.stdout.is_none() {
        o.stdout = Some(stdout());
    }
    if o.stderr.is_none() {
        o.stderr = Some(stderr());
    }
    if o.ui.is_none() {
        o.ui = Some(Arc::new(crate::ui::Console::new()));
    }
    if o.meta.is_none() {
        if o.code.is_some() {
            o.meta = Some(Meta::default());
        } else {
            let (m, warnings) = crate::meta::parse_file(&o.script_path)?;
            let mut e = o.stderr.as_ref().unwrap().lock().unwrap();
            for w in warnings {
                let _ = writeln!(e, "{w}");
            }
            o.meta = Some(m);
        }
    }
    match &o.code {
        Some(c) => Ok(c.clone()),
        None => bundle_with(&o.script_path, o.debug_script),
    }
}

/// Runs the script and returns its exit code. An error describes why the
/// script failed (the exit code is then 1).
pub fn run(mut o: Options) -> Result<i32> {
    let code = prepare(&mut o)?;
    let script = abs_path(&o.script_path);
    let meta = o.meta.clone().unwrap_or_default();
    let assets: Arc<dyn AssetFs> = match &o.assets {
        Some(a) => a.clone(),
        None => Arc::new(meta.asset_fs(&script_dir(&script))),
    };
    let env = Env {
        ui: o.ui.clone().unwrap(),
        args: o.args.clone(),
        script,
        version: o.version.clone(),
        stdout: o.stdout.clone().unwrap(),
        stderr: o.stderr.clone().unwrap(),
        title: meta.title.clone(),
        meta,
        assets: Some(assets),
        debug: o.debug,
    };
    if o.debug_script {
        return debug::run(&o, env, code);
    }
    let code = String::from_utf8_lossy(&code).into_owned();

    let rt = rquickjs::Runtime::new().map_err(|e| err!("engine.internalError", e))?;
    // The script thread of ui::run has a large stack; other threads (tests)
    // have the default one.
    let big = std::thread::current().name() == Some("script");
    rt.set_max_stack_size(if big { 32 << 20 } else { 1 << 20 });
    let ctx = rquickjs::Context::full(&rt).map_err(|e| err!("engine.internalError", e))?;
    let host = Rc::new(Host::new(env, Mapper::from_bundle(&code)));
    {
        let h = Rc::downgrade(&host);
        rt.set_host_promise_rejection_tracker(Some(Box::new(move |ctx, promise, reason, handled| {
            if let Some(h) = h.upgrade() {
                h.track_rejection(&ctx, promise, reason, handled);
            }
        })));
    }
    let res = ctx.with(|ctx| run_in(&ctx, &host, &code));
    host.env.ui_cleanup(&host);
    ctx.with(|_| host.clear());
    rt.set_host_promise_rejection_tracker(None);
    drop(ctx);
    drop(host);
    res
}

fn run_in<'js>(ctx: &Ctx<'js>, host: &Rc<Host>, code: &str) -> Result<i32> {
    install_stack_mapper(ctx, host).map_err(|e| err!("engine.internalError", e))?;
    host.install_loop(ctx).map_err(|e| err!("engine.internalError", e))?;
    let natives = match api::install(ctx, host) {
        Ok(n) => n,
        Err(e) => return Err(err!("engine.internalError", describe_error(ctx, e))),
    };
    let mut opts = rquickjs::context::EvalOptions::default();
    opts.global = true;
    opts.strict = false;
    opts.filename = Some(BUNDLE_NAME.into());
    let main: Value = match ctx.eval_with_options(code.as_bytes().to_vec(), opts).catch(ctx) {
        Ok(v) => v,
        Err(e) => return Err(script_error(describe_caught(ctx, e))),
    };
    // An app opens its main window before the script runs; the script ends
    // when the window is closed (the window keeps the event loop busy).
    if host.env.meta.is_app() {
        let open: Function = natives.get("openMain").map_err(|_| err!("engine.internalError", "openMain"))?;
        if let Err(e) = open.call::<_, Value>(()).catch(ctx) {
            if host.exited() {
                return Ok(host.exit_code());
            }
            return Err(script_error(describe_caught(ctx, e)));
        }
    }
    let Some(main) = main.as_function().cloned() else {
        return Err(err!("engine.bundleNotFunction"));
    };
    let hook: Value = natives.get("setExports").unwrap_or(Value::new_undefined(ctx.clone()));
    let result = main.call::<_, Value>((hook,)).catch(ctx);
    let result = match result {
        Ok(v) => v,
        Err(e) => {
            if host.exited() {
                return Ok(host.exit_code());
            }
            return Err(script_error(describe_caught(ctx, e)));
        }
    };
    let promise = result.as_promise().cloned();
    if let Some(p) = &promise {
        host.forget_rejection(p.as_value());
    }
    if let Err(e) = host.lp.run(ctx, host).catch(ctx) {
        if host.exited() {
            return Ok(host.exit_code());
        }
        return Err(script_error(describe_caught(ctx, e)));
    }
    if host.exited() {
        return Ok(host.exit_code());
    }
    if let Some(p) = promise {
        host.forget_rejection(p.as_value());
        match p.state() {
            rquickjs::promise::PromiseState::Rejected => {
                let v: Value = p.result::<Value>().and_then(|r| r.err()).map(|e| match e {
                    rquickjs::Error::Exception => ctx.catch(),
                    _ => Value::new_undefined(ctx.clone()),
                }).unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                return Err(script_error(describe_value(&v)));
            }
            rquickjs::promise::PromiseState::Pending => return Err(script_error(t!("engine.notFinished"))),
            _ => {}
        }
    }
    if let Some(reason) = host.first_unhandled(ctx) {
        return Err(script_error(t!("engine.unhandledRejection", describe_value(&reason))));
    }
    Ok(0)
}

/// Installs Error.prepareStackTrace: frames of the bundle are mapped to the
/// script's files with the source map; the module loader's frames are left
/// out. Every stack starts with "Name: message", like V8's.
fn install_stack_mapper<'js>(ctx: &Ctx<'js>, host: &Rc<Host>) -> rquickjs::Result<()> {
    let h = Rc::downgrade(host);
    let map = Function::new(ctx.clone(), move |file: String, line: u32, col: u32| -> Option<String> {
        if file == BUNDLE_NAME {
            let h = h.upgrade()?;
            let (src, l, c) = h.mapper.as_ref()?.lookup(line, col)?;
            return Some(format!("{src}:{l}:{c}"));
        }
        Some(format!("{file}:{line}:{col}"))
    })?;
    let install: Function = ctx.eval(STACK_MAPPER)?;
    install.call::<_, ()>((map,))
}

const STACK_MAPPER: &str = r#"(map) => {
  Error.stackTraceLimit = 30;
  Error.prepareStackTrace = (err, sites) => {
    let head;
    try { head = String(err); } catch { head = "Error"; }
    const lines = [];
    for (const s of sites) {
      const name = s.getFunctionName() || "<anonymous>";
      if (s.isNative()) { lines.push(`    at ${name} (native)`); continue; }
      const file = s.getFileName();
      if (file === undefined || file === null) continue;
      const at = map(String(file), s.getLineNumber() | 0, s.getColumnNumber() | 0);
      if (at === null || at === undefined) continue;
      lines.push(`    at ${name} (${at})`);
    }
    return lines.length ? head + "\n" + lines.join("\n") : head;
  };
}"#;

/// Describes an error thrown by a call into the engine.
pub(crate) fn describe_error(ctx: &Ctx, e: rquickjs::Error) -> String {
    match e {
        rquickjs::Error::Exception => describe_value(&ctx.catch()),
        other => i18n::text(&other.to_string()),
    }
}

pub(crate) fn describe_caught(_ctx: &Ctx, e: rquickjs::CaughtError) -> String {
    match e {
        rquickjs::CaughtError::Exception(ex) => describe_value(ex.as_value()),
        rquickjs::CaughtError::Value(v) => describe_value(&v),
        rquickjs::CaughtError::Error(e) => i18n::text(&e.to_string()),
    }
}

/// The message of a thrown value: its stack (with the message) for errors.
pub fn describe_value(v: &Value) -> String {
    if v.is_undefined() {
        return "undefined".into();
    }
    if let Some(obj) = v.as_object() {
        let stack: Option<String> = obj.get::<_, Option<String>>("stack").ok().flatten();
        if let Some(s) = stack.filter(|s| !s.is_empty()) {
            let mut s = s;
            let msg: Option<String> = obj.get::<_, Option<String>>("message").ok().flatten();
            if let Some(m) = msg
                && !s.contains(&m)
            {
                s = format!("{}\n{s}", to_string(v));
            }
            return i18n::text(&clean_stack(&s));
        }
    }
    i18n::text(&to_string(v))
}

/// String(v), "" when that throws.
pub fn to_string(v: &Value) -> String {
    let ctx = v.ctx();
    let f: Option<Function> = ctx.globals().get("String").ok();
    f.and_then(|f| f.call::<_, String>((v.clone(),)).ok()).unwrap_or_default()
}

static NATIVE_FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*at .*\(native\)\s*$\n?").unwrap());
static PRELUDE_FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*at .*xljs:prelude.*$\n?").unwrap());

/// Removes internal frames so the stack trace only shows the user's files.
pub fn clean_stack(s: &str) -> String {
    let s = NATIVE_FRAME.replace_all(s, "");
    let s = PRELUDE_FRAME.replace_all(&s, "");
    s.trim_end_matches('\n').to_string()
}

/// The natives object (tests).
#[allow(dead_code)]
pub(crate) fn natives_of<'js>(ctx: &Ctx<'js>, host: &Host) -> Option<Object<'js>> {
    host.natives(ctx)
}

#[cfg(test)]
mod tests;
