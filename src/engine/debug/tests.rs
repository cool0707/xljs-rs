//! The debugger with a fake page: a second QuickJS runtime stands in for
//! WebView2. It loads the same scripts as the debugger window, calls Rust
//! through the window messages and receives the events, presses Run as soon
//! as the page is ready and Close when the run has ended.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use base64::Engine as _;
use rquickjs::function::{Rest, This};
use rquickjs::{Ctx, Function, Object, Value};

use super::*;
use crate::engine::loop_::Shared;
use crate::engine::tests::{run_script_in, test_scripts};
use crate::testutil::{lock_lang, repo_path};
use crate::ui::{ASSET_ORIGIN, FormOptions, Mock, PickerOptions, Progress, ProgressOptions, TableOptions, Ui};

/// What broke in fake pages (not in the scripts).
static PAGE_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub(crate) fn use_fake_page(o: &mut Options) {
    o.debug = true;
    o.debug_script = true;
    o.debug_auto_run = true;
    let inner = o.ui.take().unwrap();
    o.ui = Some(Arc::new(FakeDebugUi { inner }));
}

struct FakeDebugUi {
    inner: Arc<dyn Ui>,
}

impl Ui for FakeDebugUi {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn alert(&self, a: &str, b: &str) {
        self.inner.alert(a, b)
    }
    fn confirm(&self, a: &str, b: &str) -> bool {
        self.inner.confirm(a, b)
    }
    fn prompt(&self, a: &str, b: &str, c: &str) -> Option<String> {
        self.inner.prompt(a, b, c)
    }
    fn open_files(&self, o: &PickerOptions) -> Vec<String> {
        self.inner.open_files(o)
    }
    fn save_file(&self, o: &PickerOptions) -> Option<String> {
        self.inner.save_file(o)
    }
    fn pick_directory(&self, o: &PickerOptions) -> Option<String> {
        self.inner.pick_directory(o)
    }
    fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress> {
        self.inner.progress(o)
    }
    fn show_error(&self, a: &str, b: &str) {
        self.inner.show_error(a, b)
    }
    fn show_in_folder(&self, p: &[String]) -> Result<()> {
        self.inner.show_in_folder(p)
    }
    fn open_path(&self, t: &str) -> Result<()> {
        self.inner.open_path(t)
    }
    fn open_window(&self, o: WindowOptions) -> Result<Arc<dyn Window>> {
        if !o.url.starts_with(DEBUG_PAGE) {
            return self.inner.open_window(o);
        }
        assert!(o.debug && o.isolated, "debugger window options");
        Ok(Page::start(o))
    }
    fn supports_windows(&self) -> bool {
        self.inner.supports_windows()
    }
    fn form(&self, o: &FormOptions) -> Option<serde_json::Map<String, serde_json::Value>> {
        self.inner.form(o)
    }
    fn table(&self, o: &TableOptions) -> Option<Vec<i64>> {
        self.inner.table(o)
    }
    fn notify(&self, a: &str, b: &str) {
        self.inner.notify(a, b)
    }
}

struct Page {
    o: WindowOptions,
    closed: AtomicBool,
    answers: Mutex<mpsc::Sender<String>>,
    shared: Mutex<Option<Arc<Shared>>>,
    listen: Arc<AtomicU64>,
}

fn page_error(e: String) {
    PAGE_ERRORS.lock().unwrap().push(e);
}

impl Page {
    fn start(o: WindowOptions) -> Arc<dyn Window> {
        let (tx, rx) = mpsc::channel();
        let p = Arc::new(Page { o, closed: AtomicBool::new(false), answers: Mutex::new(tx), shared: Mutex::new(None), listen: Default::default() });
        let (ready_tx, ready_rx) = mpsc::channel();
        let p2 = p.clone();
        std::thread::Builder::new().stack_size(16 << 20).spawn(move || p2.run(rx, ready_tx)).unwrap();
        ready_rx.recv().unwrap();
        p
    }

    fn run(self: Arc<Self>, answers: mpsc::Receiver<String>, ready: mpsc::Sender<()>) {
        let rt = rquickjs::Runtime::new().unwrap();
        rt.set_max_stack_size(4 << 20);
        let ctx = rquickjs::Context::full(&rt).unwrap();
        let env = Env {
            ui: Arc::new(Mock::from_json("", None).unwrap()),
            args: vec![],
            script: "page".into(),
            version: String::new(),
            stdout: Arc::new(Mutex::new(std::io::sink())),
            stderr: Arc::new(Mutex::new(std::io::sink())),
            title: String::new(),
            meta: Default::default(),
            assets: None,
            debug: false,
        };
        let host = Rc::new(Host::new(env, None));
        *self.shared.lock().unwrap() = Some(host.lp.shared.clone());
        host.lp.shared.add_pending(); // until the window closes
        let _ = ready.send(());
        let session_id: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        {
            let (h, sid) = (Rc::downgrade(&host), session_id.clone());
            rt.set_host_promise_rejection_tracker(Some(Box::new(move |ctx, promise, reason, handled| {
                let Some(h) = h.upgrade() else { return };
                if sid.get() == 0 {
                    return;
                }
                let Some(s) = h.kept(&ctx, sid.get()) else { return };
                let Some(so) = s.as_object() else { return };
                let name = if handled { "handled" } else { "rejected" };
                if let Ok(f) = so.get::<_, Function>(name) {
                    let _ = f.call::<_, Value>((This(so.clone()), promise, reason));
                }
            })));
        }
        let me = self.clone();
        ctx.with(|ctx| {
            host.install_loop(&ctx).unwrap();
            if let Err(e) = me.setup(&ctx, &host, answers, &session_id) {
                me.fail(&ctx, &host, &session_id, e);
                return;
            }
            // Uncaught errors (in timers and event handlers) are reported
            // like the page's "error" event does, and the page keeps running.
            loop {
                match host.lp.run(&ctx, &host) {
                    Ok(()) => break,
                    Err(e) => me.fail(&ctx, &host, &session_id, e),
                }
            }
        });
        ctx.with(|_| host.clear());
        rt.set_host_promise_rejection_tracker(None);
    }

    fn fail<'js>(&self, ctx: &Ctx<'js>, host: &Host, sid: &Rc<Cell<u64>>, e: rquickjs::Error) {
        let ex = match e {
            rquickjs::Error::Exception => ctx.catch(),
            other => {
                page_error(other.to_string());
                (self.o.on_message.as_ref().unwrap())(DEBUG_CLOSE.into(), vec![]);
                return;
            }
        };
        if sid.get() != 0
            && let Some(s) = host.kept(ctx, sid.get()).and_then(|s| s.into_object())
            && let Ok(f) = s.get::<_, Function>("fatal")
            && f.call::<_, Value>((This(s.clone()), ex.clone())).is_ok()
        {
            return;
        }
        page_error(super::super::describe_value(&ex));
        (self.o.on_message.as_ref().unwrap())(DEBUG_CLOSE.into(), vec![]);
    }

    fn setup<'js>(self: &Arc<Self>, ctx: &Ctx<'js>, host: &Rc<Host>, answers: mpsc::Receiver<String>, sid: &Rc<Cell<u64>>) -> rquickjs::Result<()> {
        let g = ctx.globals();
        g.set(
            "btoa",
            Function::new(ctx.clone(), |s: String| {
                let b: Vec<u8> = s.chars().map(|c| c as u32 as u8).collect();
                base64::engine::general_purpose::STANDARD.encode(b)
            })?,
        )?;
        g.set(
            "atob",
            Function::new(ctx.clone(), |s: String| {
                let b = base64::engine::general_purpose::STANDARD.decode(s).unwrap_or_default();
                b.into_iter().map(|c| c as char).collect::<String>()
            })?,
        )?;
        // Stacks as V8 writes them ("Name: message" first).
        ctx.eval::<(), _>(
            r#"Error.prepareStackTrace = (e, sites) => { let h; try { h = String(e); } catch { h = "Error"; }
  return [h, ...sites.map((s) => `    at ${s.getFunctionName() || "<anonymous>"} (${s.isNative() ? "native" : `${s.getFileName()}:${s.getLineNumber()}:${s.getColumnNumber()}`})`)].join("\n"); };"#,
        )?;
        let transport = Object::new(ctx.clone())?;
        let me = self.clone();
        transport.set(
            "call",
            Function::new(ctx.clone(), move |text: String| -> String {
                // As debug-page.js: a message, then wait for the answer.
                (me.o.on_message.as_ref().unwrap())(format!("{DEBUG_CALL}{text}"), vec![]);
                answers.recv().unwrap_or_else(|_| r#"{"stop":true}"#.into())
            })?,
        )?;
        let (h, listen) = (Rc::downgrade(host), self.listen.clone());
        transport.set(
            "listen",
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, f: Function<'js>| {
                if let Some(h) = h.upgrade() {
                    listen.store(h.keep_fn(&ctx, f), Ordering::SeqCst);
                }
            })?,
        )?;
        let hooks = Object::new(ctx.clone())?;
        let me = self.clone();
        hooks.set(
            "status",
            Function::new(ctx.clone(), move |args: Rest<Value<'js>>| {
                let state = args.0.first().map(crate::api::js::to_str).unwrap_or_default();
                if state == "done" {
                    // The user presses Close.
                    let me = me.clone();
                    std::thread::spawn(move || (me.o.on_message.as_ref().unwrap())(DEBUG_CLOSE.into(), vec![]));
                }
            })?,
        )?;
        let assets = self.o.assets.clone().unwrap();
        for name in ["debug/prelude.js", "debug/bundle.js", "debug.js"] {
            let src = assets.read(&format!("{BUILTIN_PREFIX}{name}")).map_err(|_| rquickjs::Error::Unknown)?;
            let mut opts = rquickjs::context::EvalOptions::default();
            opts.global = true;
            opts.strict = false;
            opts.filename = Some(format!("{ASSET_ORIGIN}{BUILTIN_PREFIX}{name}"));
            ctx.eval_with_options::<Value, _>(src, opts)?;
        }
        let dbg: Object = g.get("xljsDebug")?;
        let start: Function = dbg.get("start")?;
        let session: Object = start.call((transport, hooks))?;
        sid.set(host.keep(ctx, session.clone().into_value()));
        let run: Function = session.get("run")?;
        run.call::<_, Value>((This(session), false))?;
        Ok(())
    }
}

impl Window for Page {
    fn post(&self, msg: String) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        if let Some(a) = msg.strip_prefix(DEBUG_ANSWER) {
            let _ = self.answers.lock().unwrap().send(a.to_string());
            return;
        }
        let Some(shared) = self.shared.lock().unwrap().clone() else { return };
        let listen = self.listen.clone();
        shared.add_pending();
        shared.post(Box::new(move |ctx, host| {
            let id = listen.load(Ordering::SeqCst);
            match host.kept_fn(ctx, id) {
                Some(f) => f.call::<_, Value>((msg,)).map(|_| ()),
                None => {
                    page_error("event before listen".into());
                    Ok(())
                }
            }
        }));
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(shared) = self.shared.lock().unwrap().clone() {
            shared.post(Box::new(|_, _| Ok(()))); // the page's pending
        }
        if let Some(f) = &self.o.on_closed {
            f();
        }
    }
}

fn check_page() {
    let errs = std::mem::take(&mut *PAGE_ERRORS.lock().unwrap());
    assert!(errs.is_empty(), "debugger page: {errs:?}");
}

#[test]
fn scripts_in_debugger() {
    test_scripts(true);
    check_page();
}

fn write_script(name: &str, src: &str) -> String {
    let p = crate::testutil::tempdir().join(name);
    std::fs::write(&p, src).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn debugger_error_stack() {
    let _l = lock_lang("ja");
    let r = run_script_in(true, &repo_path("testdata/scripts/_errors.js").to_string_lossy(), "", &[]);
    let msg = r.err.as_ref().map(|e| e.to_string()).unwrap_or_default();
    assert!(r.code == 1 && r.err.is_some(), "want failure, got {} {msg}", r.code);
    for want in ["RangeError: boom from lib", "at fail (_errlib.js:2:", "_errors.js:3:"] {
        assert!(msg.contains(want), "error {msg:?} does not contain {want:?}");
    }
    for unwanted in ["bundle.js", "debug.js", "prelude", ASSET_ORIGIN] {
        assert!(!msg.contains(unwanted), "error {msg:?} contains {unwanted:?}");
    }
    check_page();
}

#[test]
fn debugger_page() {
    let _l = lock_lang("ja");
    let cases: &[(&str, &str, i32, &str, &str)] = &[
        ("exit", r#"console.log("a"); setTimeout(() => console.log("never"), 50); exit(3); console.log("b");"#, 3, "", "a\n"),
        ("throw", "function f() { throw new RangeError(\"だめ\"); }\nsetTimeout(() => f(), 1);", 1, "RangeError: だめ", ""),
        ("rejected", r#"await sleep(1); throw new TypeError("x");"#, 1, "TypeError: x", ""),
        ("unhandled", r#"Promise.reject(new Error("y")); await sleep(5);"#, 1, "処理されなかった Promise の reject: Error: y", ""),
        ("pending", "await new Promise(() => {});", 1, "完了しませんでした", ""),
        (
            "values",
            r#"
const b = workbooks.create();
const s = b.addSheet("S");
s.cell("A1").value = new Date(2026, 3, 1, 9, 30);
s.cell("A2").value = -0;
s.cell("A3").value = "日本語";
const v = s.cell("A1").value;
console.log(v instanceof Date, v.getFullYear(), v.getHours(), s.cell("A3").value);
const bytes = b.toArrayBuffer();
console.log(bytes instanceof ArrayBuffer, bytes.byteLength > 100);
const b2 = workbooks.open(new Uint8Array(bytes));
console.log(b2.sheetNames.join(","));
try { fs.readText("nope.txt"); } catch (e) { console.log(e.name, e.message.length > 0); }
console.log(new Blob(["x"]) instanceof Blob, typeof Intl.NumberFormat);
"#,
            0,
            "",
            "true 2026 9 日本語\ntrue true\nS\nError true\ntrue function\n",
        ),
    ];
    for (name, src, code, err, stdout) in cases {
        let r = run_script_in(true, &write_script("main.js", src), "", &[]);
        let msg = r.err.as_ref().map(|e| e.to_string()).unwrap_or_default();
        assert_eq!(r.code, *code, "{name}: err {msg}\nstderr:\n{}", r.stderr);
        assert!((err.is_empty() == msg.is_empty()) && msg.contains(err), "{name}: error {msg:?}, want {err:?}");
        if !stdout.is_empty() {
            assert_eq!(r.stdout, *stdout, "{name}");
        }
        assert!(!msg.contains(ASSET_ORIGIN), "{name}: error keeps the page URL: {msg}");
    }
    check_page();
}

#[test]
fn debugger_app_window() {
    use crate::engine::tests::SlowUi;
    use std::time::{Duration, Instant};
    let _l = lock_lang("ja");
    let app = "// ==xljs==\n// @mode app\n// ==/xljs==\n";
    let run = |u: Arc<SlowUi>, src: &str| {
        let o = crate::testutil::SharedBuf::default();
        let mut opts = Options {
            script_path: write_script("a.js", src),
            ui: Some(u),
            stdout: Some(Arc::new(Mutex::new(o.clone()))),
            stderr: Some(Arc::new(Mutex::new(std::io::sink()))),
            ..Default::default()
        };
        use_fake_page(&mut opts);
        let r = crate::engine::run(opts).map_err(|e| e.to_string());
        check_page();
        (r, o.text())
    };
    let start = Instant::now();
    let (r, out) = run(Arc::new(SlowUi::new(Some(Duration::from_millis(100)), false)), &format!("{app}app.main.closed.then(() => console.log('closed')); console.log('end of script')"));
    assert!(r == Ok(0) && start.elapsed() >= Duration::from_millis(100) && out == "end of script\nclosed\n", "{r:?} {out:?}");

    let u = Arc::new(SlowUi::new(None, false));
    let (r, _) = run(u.clone(), &format!("{app}app.open({{ title: 'sub' }}); setTimeout(() => exit(5), 10)"));
    assert_eq!(r, Ok(5));
    assert_eq!(u.events.lock().unwrap().join(","), "open a,open sub,closed by script,closed by script");

    for src in [
        "await app.open({ title: 'sub' }).closed; exit(7); console.log('after')",
        "app.open({ title: 'sub' }).closed.then(() => { exit(7); console.log('after') })",
    ] {
        let (r, out) = run(Arc::new(SlowUi::new(Some(Duration::from_millis(20)), false)), src);
        assert!(r == Ok(7) && out.is_empty(), "{src}: {r:?} {out:?}");
    }
}

