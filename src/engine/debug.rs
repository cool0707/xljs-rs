//! The debugger (xljs --debug): the prelude and the script run in the
//! debugger window (its worker), where the developer tools of WebView2
//! debug them, and call the natives here through api::Remote. This side
//! keeps the event loop, so that windows, progress and the end of the run
//! behave as they do in QuickJS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use rquickjs::Ctx;
use serde::Deserialize;
use serde_json::json;

use super::{Mapper, Options, ScriptError, clean_stack};
use crate::api::remote::Remote;
use crate::api::{Env, Host, prelude_source};
use crate::meta::{AssetFs, MemAssets};
use crate::ui::{BUILTIN_PREFIX, BuiltinAssets, Window, WindowOptions};
use crate::{Error, Result, err, i18n, t};

/// The URL of the debugger window (ui/web/debug.html).
pub const DEBUG_PAGE: &str = "https://app.xljs/_xljs/debug.html";

// Messages of the debugger window: its close button, its calls (CALL +
// request) and their answers (ANSWER + response), and with ?autorun
// (tests) how far the page got (TRACE + text). Events are JSON objects.
pub(crate) const DEBUG_CLOSE: &str = "close";
pub(crate) const DEBUG_CALL: &str = "call:";
pub(crate) const DEBUG_ANSWER: &str = "R";
pub(crate) const DEBUG_TRACE: &str = "trace:";

const UNMAPPED: &str = "\u{0}unmapped\u{0}";
const PRELUDE_PREFIX: &str = "globalThis.__xljsPrelude = ";
const BUNDLE_PREFIX: &str = "globalThis.__xljsMain = ";

type TraceFn = Arc<dyn Fn(String) + Send + Sync>;

struct Session {
    remote: Remote,
    window: RefCell<Option<Arc<dyn Window>>>,
    mapper: Option<Mapper>,
    // The page's last idle report: the number of events it had handled
    // (-1 = it has called in since), whether the script's function has
    // settled and how the script failed.
    idle_at: Cell<i64>,
    settled: Cell<bool>,
    failure: RefCell<Option<String>>,
    trace: TraceFn,
    started: Cell<bool>,
    done: Cell<bool>,
    code: Cell<i32>,
    err: RefCell<Option<String>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Request {
    op: String,
    name: String,
    #[serde(rename = "ref")]
    reference: i64,
    args: Vec<serde_json::Value>,
    code: i32,
    seen: i64,
    settled: bool,
    error: Option<String>,
}

fn session(host: &Host) -> Option<Rc<Session>> {
    host.ext.borrow().clone().and_then(|e| e.downcast::<Session>().ok())
}

/// Cuts s for the trace.
fn short(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() > 100 { format!("{}…", c[..100].iter().collect::<String>()) } else { s.to_string() }
}

fn error_response(msg: &str) -> String {
    json!({"error": {"$err": {"name": "Error", "message": msg}}}).to_string()
}

impl Session {
    fn post(&self, msg: String) {
        if let Some(w) = &*self.window.borrow() {
            w.post(msg);
        }
    }

    /// Answers a request of the page (debug.js).
    fn handle(&self, ctx: &Ctx, host: &Host, body: &str) -> String {
        (self.trace)(format!("request {}", short(body)));
        let r: Request = match serde_json::from_str(body) {
            Ok(r) => r,
            Err(e) => return error_response(&e.to_string()),
        };
        if self.done.get() {
            // The page stops when it gets the "done" event; until then it
            // may still call in.
            return r#"{"stop":true}"#.into();
        }
        match r.op.as_str() {
            "init" => {
                // A reloaded page would run the script again on the same natives.
                if self.started.get() {
                    return error_response(&t!("debug.reloaded"));
                }
                self.started.set(true);
                return json!({"value": self.remote.manifest(ctx)}).to_string();
            }
            "call" => {
                self.idle_at.set(-1);
                return self.remote.call(ctx, &r.name, &r.args).to_string();
            }
            "invoke" => {
                self.idle_at.set(-1);
                return self.remote.invoke(ctx, r.reference, &r.args).to_string();
            }
            "idle" => {
                self.idle_at.set(r.seen);
                self.settled.set(r.settled);
                *self.failure.borrow_mut() = r.error;
            }
            "exit" => self.finish(host, r.code, None),
            "fail" => {
                let msg = self.page_error(r.error.as_deref().unwrap_or(""));
                self.finish(host, 1, Some(msg));
            }
            _ => return error_response(&format!("xljs: unknown request {}", r.op)),
        }
        r#"{"value":{"$u":0}}"#.into()
    }

    /// Ends the run when the page is idle (it has handled every event) and
    /// nothing but the debugger window is pending.
    fn check(&self, host: &Host) {
        if self.done.get() || self.idle_at.get() < 0 || self.idle_at.get() != self.remote.sent() as i64 || !host.lp.shared.idle(1) {
            return;
        }
        let failure = self.failure.borrow().clone();
        match failure {
            Some(f) => {
                let msg = self.page_error(&f);
                self.finish(host, 1, Some(msg));
            }
            None if !self.settled.get() => self.finish(host, 1, Some(t!("engine.notFinished"))),
            None => self.finish(host, 0, None),
        }
    }

    /// Ends the run: what the script left open is closed and the page shows
    /// the result. The debugger window stays open until the user closes it.
    fn finish(&self, host: &Host, code: i32, err: Option<String>) {
        if self.done.get() {
            return;
        }
        self.done.set(true);
        self.code.set(code);
        (self.trace)(format!("finish {code} {}", err.as_deref().unwrap_or("")));
        *self.err.borrow_mut() = err.clone();
        host.cleanup();
        self.post(json!({"ev": "done", "code": code, "error": err.unwrap_or_default()}).to_string());
    }

    /// The debugger window has closed. A script still running is stopped.
    fn closed(&self, host: &Host) {
        if self.done.get() {
            return;
        }
        self.done.set(true);
        self.code.set(130);
        (self.trace)("closed while running".into());
        let mut e = host.env.stderr.lock().unwrap();
        let _ = writeln!(e, "{}", t!("debug.aborted"));
        drop(e);
        host.cleanup();
    }

    /// Makes an error of the page look like one from QuickJS: frames of the
    /// bundle point to the script's files (with the bundle's source map)
    /// and the page's own frames are left out.
    fn page_error(&self, msg: &str) -> String {
        let msg = BUNDLE_FRAME.replace_all(msg, |c: &regex::Captures| {
            let line: u32 = c[1].parse().unwrap_or(0);
            let mut col: i64 = c[2].parse().unwrap_or(0);
            if line == 1 {
                col -= BUNDLE_PREFIX.len() as i64;
            }
            if let Some(m) = &self.mapper
                && let Some((src, l, cc)) = m.lookup(line, col.max(1) as u32)
            {
                return format!("{src}:{l}:{cc}");
            }
            // The bundle's module loader: not the script's code.
            UNMAPPED.to_string()
        });
        let msg: String = msg.split_inclusive('\n').filter(|l| !l.contains(UNMAPPED)).collect();
        let msg = EVAL_FRAME.replace_all(&msg, "at $1");
        let msg = PAGE_FRAME.replace_all(&msg, "");
        i18n::text(&clean_stack(&msg))
    }
}


static BUNDLE_FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"[^\s(]*{}:(\d+):(\d+)", regex::escape(&format!("{BUILTIN_PREFIX}debug/bundle.js")))).unwrap());
// The worker evaluates the bundle: its top level shows as "eval".
static EVAL_FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"at eval \(([^()\s]+)\)").unwrap());
static PAGE_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?m)^\s*at .*{}(?:debug\.js|debug-page\.js|debug-worker\.js|debug/prelude\.js).*$\n?",
        regex::escape(BUILTIN_PREFIX)
    ))
    .unwrap()
});

/// The files of the debugger window: the built-in pages, the prelude and the
/// bundled script (as functions that the page runs).
pub(crate) struct DebugAssets(MemAssets);

impl AssetFs for DebugAssets {
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        if let Ok(d) = self.0.read(name) {
            return Ok(d);
        }
        match name.strip_prefix(BUILTIN_PREFIX) {
            Some(rest) => BuiltinAssets.read(rest),
            None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string())),
        }
    }
    fn files(&self, root: &str) -> std::io::Result<Vec<String>> {
        self.0.files(root)
    }
}

fn debug_assets(code: &[u8]) -> DebugAssets {
    let mut m = MemAssets::new();
    m.insert(format!("{BUILTIN_PREFIX}debug/prelude.js"), format!("{PRELUDE_PREFIX}{}", prelude_source()));
    let mut b = BUNDLE_PREFIX.as_bytes().to_vec();
    b.extend_from_slice(code);
    m.insert(format!("{BUILTIN_PREFIX}debug/bundle.js"), b);
    DebugAssets(m)
}

pub(super) fn run(o: &Options, env: Env, code: Vec<u8>) -> Result<i32> {
    let trace: TraceFn = o.debug_trace.clone().unwrap_or_else(|| Arc::new(|_| {}));
    let rt = rquickjs::Runtime::new().map_err(|e| err!("engine.internalError", e))?;
    let ctx = rquickjs::Context::full(&rt).map_err(|e| err!("engine.internalError", e))?;
    let code_text = String::from_utf8_lossy(&code).into_owned();
    let mut env = env;
    env.debug = true;
    let host = Rc::new(Host::new(env, None));
    let res = ctx.with(|ctx| -> Result<i32> {
        host.install_loop(&ctx).map_err(|e| err!("engine.internalError", e))?;
        let remote = Remote::new(&ctx, &host).map_err(|e| err!("engine.internalError", e))?;
        let s = Rc::new(Session {
            remote: remote.clone(),
            window: RefCell::new(None),
            mapper: Mapper::from_bundle(&code_text),
            idle_at: Cell::new(-1),
            settled: Cell::new(false),
            failure: RefCell::new(None),
            trace: trace.clone(),
            started: Cell::new(false),
            done: Cell::new(false),
            code: Cell::new(0),
            err: RefCell::new(None),
        });
        *host.ext.borrow_mut() = Some(s.clone());
        {
            let w = Rc::downgrade(&host);
            *host.exit_hook.borrow_mut() = Some(Box::new(move |code| {
                if let Some(h) = w.upgrade()
                    && let Some(s) = session(&h)
                {
                    s.finish(&h, code, None);
                }
            }));
        }
        *host.lp.after.borrow_mut() = Some(Box::new(|h: &Host| {
            if let Some(s) = session(h) {
                s.check(h);
            }
        }));

        // The debugger window keeps the loop running until it is closed.
        let poster = host.poster();
        poster.add_pending();
        let mut page = DEBUG_PAGE.to_string();
        if o.debug_auto_run {
            page.push_str("?autorun");
        }
        let (p1, p2, t1, t2) = (poster.clone(), poster.clone(), trace.clone(), trace.clone());
        let w = host.env.ui.open_window(WindowOptions {
            title: t!("debug.windowTitle", host.env.title),
            width: 600,
            height: 300,
            url: page,
            assets: Some(Arc::new(debug_assets(&code))),
            debug: true,
            dev_tools: !o.debug_auto_run,
            isolated: true,
            trace: Some(Arc::new(move |s: String| t2(format!("window {s}")))),
            on_message: Some(Arc::new(move |msg: String, _files: Vec<String>| {
                if let Some(t) = msg.strip_prefix(DEBUG_TRACE) {
                    t1(format!("page {t}"));
                    return;
                }
                if let Some(req) = msg.strip_prefix(DEBUG_CALL) {
                    // A call of the page, which waits for the answer.
                    let req = req.to_string();
                    p1.add_pending();
                    p1.post(Box::new(move |ctx, host| {
                        if let Some(s) = session(host) {
                            let res = s.handle(ctx, host, &req);
                            (s.trace)(format!("answer {}", short(&res)));
                            s.post(format!("{DEBUG_ANSWER}{res}"));
                        }
                        Ok(())
                    }));
                    return;
                }
                if msg == DEBUG_CLOSE {
                    p1.add_pending();
                    p1.post(Box::new(|_, host| {
                        if let Some(s) = session(host)
                            && let Some(w) = s.window.borrow().clone()
                        {
                            w.close();
                        }
                        Ok(())
                    }));
                }
            })),
            on_closed: Some(Arc::new(move || {
                p2.post(Box::new(|_, host| {
                    if let Some(s) = session(host) {
                        s.closed(host);
                    }
                    Ok(())
                }));
            })),
            ..Default::default()
        })?;
        *s.window.borrow_mut() = Some(w.clone());
        let t3 = trace.clone();
        remote.set_send(move |ev| {
            t3(format!("event {}", short(&ev)));
            w.post(ev);
        });
        if let Err(e) = host.lp.run(&ctx, &host) {
            return Err(Error::Script(ScriptError { message: super::describe_error(&ctx, e) }));
        }
        let code = s.code.get();
        let err = s.err.borrow().clone();
        remote.clear();
        match err {
            Some(m) => Err(Error::Script(ScriptError { message: m })),
            None => Ok(code),
        }
    });
    host.cleanup();
    ctx.with(|_| host.clear());
    drop(ctx);
    drop(host);
    res
}

#[cfg(test)]
pub(crate) mod tests;
