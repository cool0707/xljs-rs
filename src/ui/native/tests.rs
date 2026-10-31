//! Native dialogs and windows on a real desktop (Windows CI): dialogs press
//! their default button by themselves.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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

#[test]
fn webview_window() {
    if webview2_version().is_empty() {
        eprintln!("WebView2 runtime not installed; skipped");
        return;
    }
    let dir = crate::testutil::tempdir();
    unsafe { std::env::set_var(crate::ui::WEBVIEW2_DATA_ENV, &dir) };
    let u = setup();
    let messages = Arc::new(Mutex::new(Vec::<String>::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let mut assets = crate::meta::MemAssets::new();
    assets.insert(
        "index.html",
        r#"<!doctype html><title>t</title><script>
chrome.webview.addEventListener("message", (e) => { if (e.data === "\u0002ping") chrome.webview.postMessage("pong"); });
chrome.webview.postMessage("ready:" + (typeof xljs));
</script>"#,
    );
    let (m2, c2) = (messages.clone(), closed.clone());
    let w = u
        .open_window(WindowOptions {
            title: "テスト".into(),
            width: 400,
            height: 300,
            url: format!("{ASSET_ORIGIN}index.html"),
            assets: Some(Arc::new(assets)),
            on_message: Some(Arc::new(move |m, _| m2.lock().unwrap().push(m))),
            on_closed: Some(Arc::new(move || c2.store(true, Ordering::SeqCst))),
            ..Default::default()
        })
        .unwrap();
    // Pump messages until the page answers (or give up).
    let start = std::time::Instant::now();
    let mut pinged = false;
    while start.elapsed() < std::time::Duration::from_secs(30) {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        let got = messages.lock().unwrap().clone();
        if !pinged && got.iter().any(|m| m.starts_with("ready:")) {
            w.post("ping".into());
            pinged = true;
        }
        if got.iter().any(|m| m == "pong") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let got = messages.lock().unwrap().clone();
    assert!(got.contains(&"ready:object".to_string()) && got.contains(&"pong".to_string()), "messages: {got:?}");
    w.close();
    assert!(closed.load(Ordering::SeqCst));
}
