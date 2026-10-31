//! Native dialogs and windows on a real desktop (Windows CI): dialogs press
//! their default button by themselves.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

fn setup() -> Arc<dyn Ui> {
    dialogs::AUTO_CLOSE.store(true, Ordering::SeqCst);
    Native::new().unwrap()
}

#[test]
fn dialogs_open_and_close() {
    let _l = crate::testutil::lock_lang("ja");
    let u = setup();
    assert_eq!(u.prompt("xljs テスト", "名前を入力してください\n2 行目", "既定値"), Some("既定値".to_string()));
    u.show_error("xljs テスト", "エラーの例\n\tat main.js:1:1");
    let choices = vec![
        Choice { label: "実行".into(), note: "スクリプトを実行します".into(), icon: include_bytes!("../../cli/icons/run.ico").to_vec() },
        Choice { label: "終了".into(), ..Default::default() },
    ];
    assert_eq!(u.choose("xljs", "どうしますか？", &choices), Some(0));
    let p = u.progress(&ProgressOptions { title: "進捗".into(), total: 3, message: "処理中".into(), cancellable: true });
    p.update(1, 3, "1 件目");
    assert!(!p.cancelled());
    p.close();
}

/// Opens a window and pumps messages until handle says done (or 30
/// seconds pass), then closes it. handle gets each message from the page
/// and the window; it returns true when the test has what it needs.
/// Returns the messages.
fn with_page(mut o: WindowOptions, mut handle: impl FnMut(&str, &Pager) -> bool) -> Vec<String> {
    if webview2_version().is_empty() {
        eprintln!("WebView2 runtime not installed; skipped");
        return vec!["skipped".into()];
    }
    static DATA: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let dir = DATA.get_or_init(crate::testutil::tempdir);
    unsafe { std::env::set_var(crate::ui::WEBVIEW2_DATA_ENV, dir) };
    let u = setup();
    let inbox = Arc::new(Mutex::new(Vec::<String>::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let (i2, c2) = (inbox.clone(), closed.clone());
    o.on_message = Some(Arc::new(move |m, _| i2.lock().unwrap().push(m)));
    o.on_closed = Some(Arc::new(move || c2.store(true, Ordering::SeqCst)));
    let w = u.open_window(o).unwrap();
    let pager = Pager { w: w.clone(), later: RefCell::new(Vec::new()) };
    let mut got = Vec::new();
    let start = Instant::now();
    'outer: while start.elapsed() < Duration::from_secs(30) {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        let due: Vec<String> = {
            let mut later = pager.later.borrow_mut();
            let (due, rest): (Vec<_>, Vec<_>) = later.drain(..).partition(|(t, _)| *t <= Instant::now());
            *later = rest;
            due.into_iter().map(|(_, m)| m).collect()
        };
        for m in due {
            w.post(m);
        }
        let msgs: Vec<String> = inbox.lock().unwrap().drain(..).collect();
        for m in msgs {
            got.push(m.clone());
            if handle(&m, &pager) {
                break 'outer;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    w.close();
    w.close();
    assert!(closed.load(Ordering::SeqCst), "on_closed was not called");
    got
}

/// The window, for with_page's handler.
struct Pager {
    w: Arc<dyn Window>,
    later: RefCell<Vec<(Instant, String)>>,
}

impl Pager {
    fn post(&self, m: &str) {
        self.w.post(m.to_string());
    }
    /// Posts m after d, pumping messages meanwhile.
    fn post_after(&self, d: Duration, m: &str) {
        self.later.borrow_mut().push((Instant::now() + d, m.to_string()));
    }
}

fn skipped(got: &[String]) -> bool {
    got == ["skipped"]
}

/// A page from the window's assets runs its scripts (and the page
/// runtime), and messages go both ways.
#[test]
fn webview_window() {
    let mut assets = crate::meta::MemAssets::new();
    assets.insert("ui/index.html", r#"<!doctype html><meta charset="utf-8"><title>t</title><script src="app.js"></script>"#);
    assets.insert(
        "ui/app.js",
        r#"chrome.webview.addEventListener("message", (e) => { if (e.data === "\u0002ping") chrome.webview.postMessage("pong"); });
chrome.webview.postMessage("ready " + location.href + " " + document.title + " " + typeof xljs);"#,
    );
    let o = WindowOptions { title: "テスト".into(), width: 400, height: 300, url: format!("{ASSET_ORIGIN}ui/index.html"), assets: Some(Arc::new(assets)), ..Default::default() };
    let got = with_page(o, |m, p| {
        if m.starts_with("ready ") {
            p.post("ping");
        }
        m == "pong"
    });
    if skipped(&got) {
        return;
    }
    let got: Vec<&String> = got.iter().filter(|m| !m.starts_with(r#"{"xljs""#)).collect();
    assert_eq!(got, ["ready https://app.xljs/ui/index.html t object", "pong"]);
}

/// What the debugger window relies on: with isolated the page is
/// cross-origin isolated, and a worker made from a blob waits with
/// Atomics.wait on a growable SharedArrayBuffer until the page wakes it.
#[test]
fn webview_isolated() {
    let mut assets = crate::meta::MemAssets::new();
    assets.insert(
        "index.html",
        r#"<!doctype html><meta charset="utf-8"><title>t</title><script>
try {
  const sab = new SharedArrayBuffer(8, { maxByteLength: 64 });
  const src = "onmessage = (e) => { const a = new Int32Array(e.data); postMessage('waiting'); Atomics.wait(a, 0, 0); postMessage('woke ' + a[1]); };";
  const w = new Worker(URL.createObjectURL(new Blob([src], { type: "text/javascript" })));
  w.onmessage = (e) => {
    if (e.data === "waiting") {
      sab.grow(16);
      const a = new Int32Array(sab);
      a[1] = 42;
      Atomics.store(a, 0, 1);
      Atomics.notify(a, 0);
    } else {
      chrome.webview.postMessage(crossOriginIsolated + " " + sab.byteLength + " " + e.data);
    }
  };
  w.onerror = (e) => chrome.webview.postMessage("worker error " + e.message);
  w.postMessage(sab);
} catch (e) {
  chrome.webview.postMessage("error " + e);
}
</script>"#,
    );
    let o = WindowOptions { title: "isolated".into(), width: 400, height: 300, url: format!("{ASSET_ORIGIN}index.html"), assets: Some(Arc::new(assets)), isolated: true, ..Default::default() };
    // The page runtime says {"xljs":"ready"} too, before or after the worker.
    let got = with_page(o, |m, _| !m.starts_with(r#"{"xljs""#));
    if skipped(&got) {
        return;
    }
    let got: Vec<&String> = got.iter().filter(|m| !m.starts_with(r#"{"xljs""#)).collect();
    assert_eq!(got, ["true 16 woke 42"]);
}

/// The form and table pages: each says "ready", renders the "init" message
/// and answers "shown".
#[test]
fn builtin_pages() {
    let inits = [
        (
            "form.html",
            r#"{"type":"init","texts":{"ok":"OK"},"fields":[{"name":"a","label":"A","type":"text","value":"x","options":[],"accept":[]},{"name":"b","label":"B","type":"files","value":[],"options":[],"accept":[".xlsx"]}]}"#,
        ),
        ("table.html", r#"{"type":"init","total":1,"select":"single","texts":{},"columns":[{"label":"A","width":0}]}"#),
    ];
    for (page, init) in inits {
        let o = WindowOptions { title: page.into(), url: format!("{ASSET_ORIGIN}{BUILTIN_PREFIX}{page}"), assets: Some(Arc::new(BuiltinOnly)), ..Default::default() };
        let got = with_page(o, |m, p| {
            if m.contains(r#""ready""#) {
                p.post(init);
            }
            m.contains(r#""shown""#)
        });
        if skipped(&got) {
            return;
        }
        assert_eq!(got.join(","), r#"{"type":"ready"},{"type":"shown"}"#, "{page}");
    }
}

/// Serves the built-in files under BUILTIN_PREFIX.
struct BuiltinOnly;

impl crate::meta::AssetFs for BuiltinOnly {
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        match name.strip_prefix(BUILTIN_PREFIX) {
            Some(rest) => BuiltinAssets.read(rest),
            None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string())),
        }
    }
    fn files(&self, _: &str) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// The injected page runtime (web/xljs.js) and the navigation handlers:
/// the page says ready, shows the state it gets with init and answers eval
/// requests; a popup and a navigation away from the app are blocked.
#[test]
fn page_runtime() {
    let mut assets = crate::meta::MemAssets::new();
    assets.insert("index.html", r#"<!doctype html><meta charset="utf-8"><title>t</title><span id="m" data-bind="msg"></span>"#);
    let requests = [
        r#"{"xljs":"eval","id":1,"code":"document.getElementById('m').textContent","fn":false,"args":[]}"#,
        r#"{"xljs":"eval","id":2,"code":"() => { window.open('about:blank'); location.href = 'file:///C:/'; return 1; }","fn":true,"args":[]}"#,
        r#"{"xljs":"eval","id":3,"code":"location.href","fn":false,"args":[]}"#,
    ];
    let o = WindowOptions { title: "runtime".into(), url: format!("{ASSET_ORIGIN}index.html"), assets: Some(Arc::new(assets)), ..Default::default() };
    let mut results = Vec::new();
    let got = with_page(o, |m, p| {
        if m == r#"{"xljs":"ready"}"# {
            p.post(r#"{"xljs":"init","state":{"msg":"こんにちは"},"handlers":[]}"#);
            p.post(requests[0]);
        } else if m.contains(r#""result""#) || m.contains(r#""error""#) {
            results.push(m.to_string());
            if results.len() < requests.len() {
                // Let a navigation start.
                p.post_after(Duration::from_millis(300), requests[results.len()]);
            }
        }
        results.len() == requests.len()
    });
    if skipped(&got) {
        return;
    }
    let want = [
        r#"{"xljs":"result","id":1,"value":"こんにちは"}"#,
        r#"{"xljs":"result","id":2,"value":1}"#,
        r#"{"xljs":"result","id":3,"value":"https://app.xljs/index.html"}"#,
    ];
    assert_eq!(results, want, "all messages: {got:?}");
}

/// Window icons from an .ico file and from a PNG image.
#[test]
fn load_icon() {
    let ico = include_bytes!("../../../assets/xljs.ico");
    let png = include_bytes!("../../../assets/xljs.png");
    for (name, data) in [("ico", &ico[..]), ("png", &png[..])] {
        for metric in [SM_CXICON, SM_CXSMICON] {
            let h = webview::icon_for_metric(data, metric);
            assert!(!h.is_invalid(), "{name}: no icon for metric {}", metric.0);
            unsafe {
                let _ = DestroyIcon(h);
            }
        }
    }
}
