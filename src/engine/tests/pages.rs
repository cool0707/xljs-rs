//! The built-in form and table pages with fake pages: a mock UI "shows"
//! forms and tables as windows, and each window opened runs the next fake
//! page, which talks to the script with the same JSON messages as
//! src/ui/web/form.js and table.js.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc;

use serde_json::{Map, Value, json};

use super::*;
use crate::ui::{ASSET_ORIGIN, BUILTIN_PREFIX, Window};

type PageFn = Box<dyn FnOnce(&FakePage) + Send>;

struct PageUi {
    m: Mock,
    windows: bool,
    pages: Mutex<VecDeque<PageFn>>,
    urls: Mutex<Vec<String>>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
    failures: Arc<Mutex<Vec<String>>>,
}

impl PageUi {
    fn new(answers: &str, log: Option<crate::ui::mock::LogSink>, pages: Vec<PageFn>) -> Arc<PageUi> {
        Arc::new(PageUi {
            m: Mock::from_json(answers, log).unwrap(),
            windows: true,
            pages: Mutex::new(pages.into()),
            urls: Default::default(),
            threads: Default::default(),
            failures: Default::default(),
        })
    }

    fn open(&self, o: WindowOptions) -> crate::Result<Arc<dyn Window>> {
        self.urls.lock().unwrap().push(o.url.clone());
        let Some(run) = self.pages.lock().unwrap().pop_front() else {
            self.failures.lock().unwrap().push(format!("unexpected window {}", o.url));
            return self.m.open_window(o);
        };
        let (tx, rx) = mpsc::channel();
        let w = Arc::new(PageWindow { o, tx: Mutex::new(Some(tx)), closed: (Mutex::new(false), std::sync::Condvar::new()) });
        let p = FakePage { w: w.clone(), rx, failures: self.failures.clone() };
        self.threads.lock().unwrap().push(std::thread::spawn(move || {
            p.send(json!({"type": "ready"}), &[]);
            run(&p);
        }));
        Ok(w)
    }

    /// Waits for the fake pages and fails with what went wrong in them.
    fn finish(&self) {
        let threads: Vec<_> = self.threads.lock().unwrap().drain(..).collect();
        for t in threads {
            if t.join().is_err() {
                self.failures.lock().unwrap().push("a fake page panicked".into());
            }
        }
        if !self.pages.lock().unwrap().is_empty() {
            self.failures.lock().unwrap().push("not every page was shown".into());
        }
        let f = self.failures.lock().unwrap();
        assert!(f.is_empty(), "fake pages:\n{}", f.join("\n"));
    }
}

forward_ui!(PageUi);

struct PageWindow {
    o: WindowOptions,
    tx: Mutex<Option<mpsc::Sender<Value>>>,
    closed: (Mutex<bool>, std::sync::Condvar),
}

impl Window for PageWindow {
    fn post(&self, msg: String) {
        let v: Value = serde_json::from_str(&msg).unwrap_or_else(|e| panic!("bad message {msg:?}: {e}"));
        if let Some(tx) = &*self.tx.lock().unwrap() {
            let _ = tx.send(v);
        }
    }
    fn close(&self) {
        let mut c = self.closed.0.lock().unwrap();
        if *c {
            return;
        }
        *c = true;
        self.tx.lock().unwrap().take();
        self.closed.1.notify_all();
        drop(c);
        if let Some(f) = &self.o.on_closed {
            f();
        }
    }
}

struct FakePage {
    w: Arc<PageWindow>,
    rx: mpsc::Receiver<Value>,
    failures: Arc<Mutex<Vec<String>>>,
}

impl FakePage {
    fn error(&self, s: String) {
        self.failures.lock().unwrap().push(s);
    }

    fn send(&self, m: Value, files: &[&Path]) {
        let files = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        (self.w.o.on_message.as_ref().unwrap())(m.to_string(), files);
    }

    /// The next message of the given type; others are skipped and
    /// collected in skipped.
    fn expect_into(&self, typ: &str, mut skipped: Option<&mut Vec<Value>>) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(m) if m["type"] == typ => return m,
                Ok(m) => {
                    if let Some(s) = skipped.as_deref_mut() {
                        s.push(m);
                    }
                }
                Err(_) => {
                    self.error(format!("timed out waiting for {typ:?}"));
                    return Value::Object(Map::new());
                }
            }
        }
    }

    fn expect(&self, typ: &str) -> Value {
        self.expect_into(typ, None)
    }

    fn wait_closed(&self) {
        let c = self.w.closed.0.lock().unwrap();
        let (c, _) = self.w.closed.1.wait_timeout_while(c, Duration::from_secs(5), |c| !*c).unwrap();
        if !*c {
            self.error("window was not closed".into());
        }
    }
}

/// JSON with the keys of objects sorted (as Go's encoding/json writes maps).
fn as_json(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                Value::Object(keys.into_iter().map(|k| (k.clone(), sort(&m[k]))).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            v => v.clone(),
        }
    }
    sort(v).to_string()
}

macro_rules! check {
    ($p:expr, $cond:expr, $($fmt:tt)*) => {
        if !$cond {
            $p.error(format!($($fmt)*));
        }
    };
}

struct Ran {
    code: i32,
    err: Option<String>,
    stdout: String,
    stderr: String,
}

/// Runs src with u (failing after 20 seconds) and waits for the pages.
fn run_pages(u: &Arc<PageUi>, name: &str, src: &str) -> Ran {
    let p = write_script(name, src);
    let (o, e) = (SharedBuf::default(), SharedBuf::default());
    let (o2, e2, u2) = (o.clone(), e.clone(), u.clone());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = run(Options { script_path: p, ui: Some(u2), stdout: Some(out(&o2)), stderr: Some(out(&e2)), version: "test".into(), ..Default::default() });
        let _ = tx.send(r.map_err(|e| e.to_string()));
    });
    let r = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| panic!("script did not finish\nstdout:\n{}\nstderr:\n{}\nfake pages:\n{}", o.text(), e.text(), u.failures.lock().unwrap().join("\n")));
    u.finish();
    let (code, err) = match r {
        Ok(c) => (c, None),
        Err(e) => (1, Some(e)),
    };
    Ran { code, err, stdout: o.text(), stderr: e.text() }
}

/// A temporary config folder (the forms remember their values there).
fn config_dir() -> std::path::PathBuf {
    let cfg = tempdir();
    unsafe { std::env::set_var("XLJS_CONFIG_DIR", &cfg) };
    cfg
}

#[test]
fn form_and_table_windows() {
    let _l = lock_lang("ja");
    let log = SharedBuf::default();
    let dir = tempdir();
    let _cfg = config_dir();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(dir.join("data/a.xlsx"), "").unwrap();
    std::fs::write(dir.join("data/b.txt"), "").unwrap();
    let data = dir.join("data");
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let (a_xlsx, b_txt) = (data.join("a.xlsx"), data.join("b.txt"));
    let d = data.clone();
    let pages: Vec<PageFn> = vec![
        // 1. Validation, picker and drop, then a valid submit closes the window.
        Box::new(move |p| {
            let init = p.expect("init");
            let got = as_json(&init["fields"]);
            check!(
                p,
                got.contains(r#""label":"フォルダ""#) && got.contains(r#""accept":[".xlsx"]"#) && got.contains(r#""value":"顧客別""#) && got.contains(r#""value":"2026-04-01""#),
                "init fields = {got}"
            );
            // Icons: a bare name is the solid icon, classes are kept (bad words dropped).
            // The form icon goes to the custom title bar (not to the page).
            check!(
                p,
                p.w.o.custom_title_bar && p.w.o.title_icon == "fa-solid fa-table" && p.w.o.title == "窓" && init["submitIcon"] == "fa-regular fa-circle-play" && init.get("icon").is_none(),
                "title bar = {} {:?} {:?}, submitIcon = {}",
                p.w.o.custom_title_bar,
                p.w.o.title_icon,
                p.w.o.title,
                init["submitIcon"]
            );
            check!(p, got.contains(r#""icon":"fa-solid fa-folder-open""#) && got.contains(r#""icon":"fa-brands fa-windows""#), "field icons = {got}");
            check!(p, init["texts"]["browse"] == "参照…", "texts = {}", init["texts"]);
            p.send(json!({"type": "submit", "values": {"src": "", "n": "x"}}), &[]);
            let e = as_json(&p.expect("errors"));
            check!(p, e.contains(r#""src":"入力してください""#) && e.contains(r#""n":"数値を入力してください""#), "errors = {e}");
            p.send(json!({"type": "pick", "field": "src", "value": ""}), &[]);
            let got = p.expect("picked");
            check!(p, got["field"] == "src" && got["value"] == path(&d) && got["append"] == false, "picked = {got}");
            p.send(json!({"type": "drop", "field": "inputs"}), &[&a_xlsx, &b_txt]);
            let got = as_json(&p.expect("picked"));
            check!(p, got.contains(r#""append":true"#) && got.contains("a.xlsx") && !got.contains("b.txt"), "dropped = {got}");
            // A folder dropped on a save field keeps the file name; a file
            // replaces it (when its type is accepted); a folder on a folder
            // field is that folder.
            p.send(json!({"type": "drop", "field": "out", "value": "集計.xlsx"}), &[&d]);
            let got = p.expect("picked");
            check!(p, got["value"] == path(&d.join("集計.xlsx")), "folder on save field = {}", as_json(&got));
            p.send(json!({"type": "drop", "field": "out", "value": ""}), &[&b_txt]);
            p.send(json!({"type": "drop", "field": "out", "value": ""}), &[&a_xlsx]);
            let got = p.expect("picked");
            check!(p, got["value"] == path(&a_xlsx), "file on save field = {}", as_json(&got));
            p.send(json!({"type": "drop", "field": "src"}), &[&d]);
            let got = p.expect("picked");
            check!(p, got["value"] == path(&d), "folder on folder field = {}", as_json(&got));
            p.send(json!({"type": "submit", "values": {"src": "data", "n": "2", "mode": "商品別", "day": "2026-05-06", "inputs": ["data/a.xlsx"]}}), &[]);
            p.wait_closed();
        }),
        // 2. onSubmit: progress and log, output folder, run again, close.
        Box::new(|p| {
            p.expect("init");
            p.send(json!({"type": "submit", "values": {"n": "3"}}), &[]);
            p.expect("running");
            let mut updates = Vec::new();
            let done = p.expect_into("done", Some(&mut updates));
            let got = as_json(&done);
            check!(p, got == r#"{"cancelled":false,"ok":true,"output":true,"type":"done"}"#, "done = {got}");
            let all = as_json(&Value::Array(updates));
            check!(p, all.contains(r#""item 1""#) && all.contains(r#""item 3""#), "log = {all}");
            check!(p, all.contains(r#""done":3,"message":"","total":3,"type":"progress""#), "progress = {all}");
            p.send(json!({"type": "open"}), &[]);
            p.send(json!({"type": "close"}), &[]);
            p.wait_closed();
        }),
        // 3. Cancelling a run.
        Box::new(|p| {
            p.expect("init");
            p.send(json!({"type": "submit", "values": {}}), &[]);
            p.expect("running");
            p.send(json!({"type": "cancel"}), &[]);
            let got = as_json(&p.expect("done"));
            check!(p, got == r#"{"cancelled":true,"ok":true,"output":false,"type":"done"}"#, "done = {got}");
            p.send(json!({"type": "close"}), &[]);
            p.wait_closed();
        }),
        // 4. An error in onSubmit is shown in the window.
        Box::new(|p| {
            p.expect("init");
            p.send(json!({"type": "submit", "values": {}}), &[]);
            let done = p.expect("done");
            let e = done["error"].as_str().unwrap_or("");
            check!(p, done["ok"] == false && e.contains("Error: 失敗") && e.contains("w.js:"), "done = {done}");
            p.send(json!({"type": "close"}), &[]);
            p.wait_closed();
        }),
        // 5. Table: rows, save as CSV, select.
        Box::new(|p| {
            let init = p.expect("init");
            let cols = as_json(&init["columns"]);
            check!(
                p,
                cols == r#"[{"label":"名前","width":0},{"label":"数","width":0},{"label":"日","width":0}]"# && init["total"] == 3 && init["select"] == "multiple",
                "init = {init}"
            );
            let rows = as_json(&p.expect("rows")["rows"]);
            check!(p, rows == r#"[["a",1,"2026-04-01"],["b",2,null],["c",null,"2026-04-01 12:30:00"]]"#, "rows = {rows}");
            p.send(json!({"type": "save", "format": "csv", "indices": [2, 0]}), &[]);
            let got = p.expect("saved");
            check!(p, got["path"].as_str().unwrap_or("").ends_with("out.csv"), "saved = {got}");
            p.send(json!({"type": "save", "format": "xlsx", "indices": null}), &[]);
            p.expect("saved");
            p.send(json!({"type": "ok", "indices": [0, 2, 7]}), &[]);
            p.wait_closed();
        }),
        // 6. Display-only table with a double-click handler that closes it.
        Box::new(|p| {
            let init = p.expect("init");
            check!(p, init["dblclick"] == true, "init = {init}");
            p.expect("rows");
            p.send(json!({"type": "dblclick", "index": 1, "column": 1}), &[]);
            p.send(json!({"type": "dblclick", "index": 2, "column": -1}), &[]);
            p.send(json!({"type": "dblclick", "index": 9, "column": 0}), &[]);
            p.wait_closed();
        }),
    ];
    let u = PageUi::new(r#"{"showDirectoryPicker": ["data"], "showSaveFilePicker": ["out.csv", "out.xlsx"]}"#, Some(log.sink()), pages);
    let src = r#"
const v = await ui.form({ title: "窓", titleBar: "custom", icon: "table", submitIcon: "fa-regular fa-circle-play", fields: [
  { name: "src", label: "フォルダ", type: "folder", required: true, icon: "folder-open" },
  { name: "n", type: "number", icon: "bad<icon> fa-brands  fa-windows" },
  { name: "mode", type: "select", options: ["顧客別", "商品別"] },
  { name: "day", type: "date", default: new Date(2026, 3, 1) },
  { name: "inputs", type: "files", accept: ".xlsx" },
  { name: "out", type: "saveFile", accept: ".xlsx", default: "集計.xlsx" },
]});
console.log("1", v.src.name, v.n, v.mode, v.day.getDate(), v.inputs.map((h) => h.name).join());

const v2 = await ui.form({ title: "実行", fields: [{ name: "n", type: "number" }], async onSubmit(values, ctx) {
  for (const i of ctx.withProgress([1, 2, 3])) ctx.log("item " + i);
  return fs.file("data/a.xlsx");
}});
console.log("2", v2.n);

const v3 = await ui.form({ title: "中止", fields: [], async onSubmit(values, ctx) {
  for (let i = 0; i < 200 && !ctx.cancelled; i++) await sleep(5);
}});
console.log("3", v3 !== null);

await ui.form({ fields: [], onSubmit() { throw new Error("失敗"); } });

const rows = [{ 名前: "a", 数: 1, 日: new Date(2026, 3, 1) }, { 名前: "b", 数: 2 }, { 名前: "c", 日: new Date(2026, 3, 1, 12, 30) }];
const picked = await ui.table(rows, { title: "表", selectable: true });
console.log("5", picked.map((r) => r.名前).join());
console.log("csv", JSON.stringify(fs.readText("out.csv")));
console.log("xlsx", workbooks.open("out.xlsx").sheets[0].toArrays().length);

const view = ui.table(rows, { onDoubleClick(e) {
  console.log("dbl", e.index, e.row.名前, e.column, e.key, e.value);
  if (e.column === null) e.close();
}});
await view.closed;
console.log("6 closed");
"#;
    let r = run_pages(&u, "w.js", src);
    std::env::set_current_dir(prev).unwrap();
    assert!(r.code == 0 && r.err.is_none(), "exit {}: {:?}\nstdout:\n{}\nstderr:\n{}\nlog:\n{}", r.code, r.err, r.stdout, r.stderr, log.text());
    let want = "1 data 2 商品別 6 a.xlsx\n2 3\n3 true\n5 a,c\n".to_string()
        + "csv \"名前,数,日\\r\\nc,,2026/04/01 12:30:00\\r\\na,1,2026/04/01\\r\\n\""
        + "\nxlsx 4\ndbl 1 b 数 数 2\ndbl 2 c null null undefined\n6 closed\n";
    assert_eq!(r.stdout, want);
    assert!(log.text().contains("showItemInFolder(["), "open folder not requested:\n{}", log.text());
    for url in u.urls.lock().unwrap().iter() {
        assert!(url.starts_with(&format!("{ASSET_ORIGIN}{BUILTIN_PREFIX}")), "window url {url}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opens windows that call the script's "dropped" with attached paths, as
/// when a folder and a file are dropped on a page.
struct AttachUi {
    m: Mock,
    windows: bool,
    files: Vec<String>,
    slow: SlowUi,
}

impl AttachUi {
    fn open(&self, o: WindowOptions) -> crate::Result<Arc<dyn Window>> {
        let f = o.on_message.clone().unwrap();
        let files = self.files.clone();
        std::thread::spawn(move || {
            f(r#"{"xljs":"ready"}"#.into(), Vec::new());
            f(r#"{"xljs":"invoke","id":1,"name":"dropped","args":[[{"$attach":0},{"$attach":1},{"$attach":2}]]}"#.into(), files);
        });
        self.slow.open(o)
    }
}

forward_ui!(AttachUi);

#[test]
fn dropped_folder() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    let file = dir.join("a.xlsx");
    std::fs::write(&file, "").unwrap();
    let u = Arc::new(AttachUi {
        m: Mock::from_json("", None).unwrap(),
        windows: false,
        files: vec![dir.to_string_lossy().into_owned(), file.to_string_lossy().into_owned(), String::new()],
        slow: SlowUi::new(None, false),
    });
    let src = "// ==xljs==\n// @mode app\n// ==/xljs==\nexport function dropped(hs) { console.log(hs.map((h) => h && h.kind).join()); app.main.close(); }\n";
    let o = SharedBuf::default();
    let r = run(Options { script_path: write_script("drop.js", src), ui: Some(u), stdout: Some(out(&o)), stderr: Some(quiet()), ..Default::default() });
    assert!(matches!(r, Ok(0)) && o.text() == "directory,file,\n", "{:?} stdout {:?}", r.map_err(|e| e.to_string()), o.text());
}

/// Runs src with the pages and checks that it ends well with stdout want.
fn form_case(name: &str, src: &str, want: &str, pages: Vec<PageFn>) {
    let _l = lock_lang("ja");
    let _cfg = config_dir();
    let u = PageUi::new("", None, pages);
    let r = run_pages(&u, name, src);
    assert!(r.code == 0 && r.err.is_none() && r.stdout == want, "exit {}, {:?}, stdout {:?}, want {want:?}\nstderr:\n{}", r.code, r.err, r.stdout, r.stderr);
}

#[test]
fn form_window_tables_and_tabs() {
    let pages: Vec<PageFn> = vec![Box::new(|p| {
        let init = p.expect("init");
        let tabs = as_json(&init["tabs"]);
        check!(p, tabs == r#"["基本","詳細"]"#, "tabs = {tabs}");
        let fields = as_json(&init["fields"]);
        for want in [
            r#""name":"common","options":[],"placeholder":"","required":false,"rows":0,"step":null,"tab":-1"#,
            r#""name":"a","#,
            r#""tab":0"#,
            r#""table":{"columns":["名前","数"],"rows":[["a",1],["b",null]],"select":"single"},"tooltip":"","type":"table","value":[1]"#,
            r#""name":"out","options":[],"placeholder":"","required":false,"rows":3,"step":null,"tab":1,"table":{"columns":[],"rows":[],"select":""}"#,
        ] {
            check!(p, fields.contains(want), "fields do not contain {want}:\n{fields}");
        }
        p.send(json!({"type": "submit", "values": {"common": "c", "a": "x", "pick": [], "out": []}}), &[]);
        let e = as_json(&p.expect("errors"));
        check!(p, e.contains(r#""pick":"行を選択してください""#), "errors = {e}");
        p.send(json!({"type": "submit", "values": {"common": "c", "a": "x", "pick": [0], "out": []}}), &[]);
        p.expect("running");
        let got = as_json(&p.expect("table"));
        check!(p, got == r#"{"columns":["結果"],"field":"out","rows":[["a"]],"select":"","type":"table"}"#, "table = {got}");
        p.expect("done");
        p.send(json!({"type": "close"}), &[]);
        p.wait_closed();
    })];
    let src = r#"
const rows = [{ 名前: "a", 数: 1 }, { 名前: "b" }];
const v = await ui.form({
  fields: [{ name: "common" }],
  tabs: [
    { label: "基本", fields: [{ name: "a" }, { name: "pick", type: "table", rows, selectable: "single", required: true, default: 1 }] },
    { label: "詳細", fields: [{ name: "out", type: "table", rows: [] }] },
  ],
  onSubmit(values, ctx) { ctx.table("out", [{ 結果: values.pick.名前 }]); },
});
console.log(v.common, v.a, v.pick.名前, "out" in v);
"#;
    form_case("tabs.js", src, "c x a false\n", pages);
}

#[test]
fn form_window_buttons() {
    let pages: Vec<PageFn> = vec![
        Box::new(|p| {
            let init = p.expect("init");
            check!(p, as_json(&init["buttons"]) == r#"["読込","確認","完了"]"# && init["noSubmit"] == false, "init = {init}");
            // validate: false runs with the values as entered; ctx.setValue updates the page.
            p.send(json!({"type": "submit", "button": 0, "values": {"n": ""}}), &[]);
            p.expect("running");
            let got = as_json(&p.expect("picked"));
            check!(p, got == r#"{"append":false,"field":"n","quiet":true,"seq":1,"type":"picked","value":"7"}"#, "picked = {got}");
            p.expect("done");
            // A checked button shows the errors and runs once they are fixed; the window stays open.
            p.send(json!({"type": "submit", "button": 1, "values": {"n": ""}}), &[]);
            let e = as_json(&p.expect("errors"));
            check!(p, e.contains(r#""n":"入力してください""#), "errors = {e}");
            p.send(json!({"type": "submit", "button": 1, "values": {"n": "7"}}), &[]);
            p.expect("running");
            let got = as_json(&p.expect("done"));
            check!(p, got == r#"{"cancelled":false,"ok":true,"output":false,"type":"done"}"#, "done = {got}");
            p.send(json!({"type": "submit", "button": 9, "values": {"n": "1"}}), &[]);
            // close: true closes the window after the run.
            p.send(json!({"type": "submit", "button": 2, "values": {"n": "8"}}), &[]);
            p.wait_closed();
        }),
        Box::new(|p| {
            let init = p.expect("init");
            check!(p, init["noSubmit"] == true, "init = {init}");
            p.send(json!({"type": "submit", "values": {}}), &[]); // the hidden submit button: ignored
            p.send(json!({"type": "submit", "button": 0, "values": {}}), &[]);
            p.expect("done");
            p.send(json!({"type": "close"}), &[]);
            p.wait_closed();
        }),
    ];
    let src = r#"
const v = await ui.form({
  fields: [{ name: "n", type: "number", required: true }],
  buttons: [
    { label: "読込", validate: false, onClick(values, ctx) { console.log("load", values.n); ctx.setValue("n", 7); } },
    { label: "確認", onClick(values) { console.log("check", values.n); } },
    { label: "完了", close: true, onClick(values) { console.log("finish", values.n); } },
  ],
});
console.log("v", v.n);
const w = await ui.form({ submit: false, fields: [], buttons: [{ label: "実行", onClick() { console.log("run"); } }] });
console.log("w", JSON.stringify(w));
"#;
    form_case("buttons.js", src, "load null\ncheck 7\nfinish 8\nv 8\nrun\nw {}\n", pages);
}

#[test]
fn form_window_editable_table() {
    let pages: Vec<PageFn> = vec![Box::new(|p| {
        let init = p.expect("init");
        let got = as_json(&init["fields"]);
        check!(
            p,
            got.contains(r#""editable":[null,{"max":null,"min":0,"options":[],"required":true,"step":null,"type":"number"},{"max":null,"min":null,"options":[{"label":"売上","value":"売上"},{"label":"返品","value":"返品"}],"required":false,"step":null,"type":"select"}]"#),
            "fields = {got}"
        );
        // The errors of the rows as shown, then of the row edited.
        let got = as_json(&p.expect("cellErrors"));
        check!(p, got == r#"{"errors":[{"col":1,"message":"数値を入力してください","row":1},{"col":-1,"message":"2 件以上","row":1}],"field":"t","type":"cellErrors"}"#, "cellErrors = {got}");
        p.send(json!({"type": "cell", "field": "t", "row": 1, "col": 1, "value": "y"}), &[]);
        let got = as_json(&p.expect("cellErrors"));
        check!(p, got == r#"{"errors":[{"col":1,"message":"数値を入力してください","row":1},{"col":-1,"message":"2 件以上","row":1}],"field":"t","row":1,"type":"cellErrors"}"#, "cellErrors = {got}");
        p.send(json!({"type": "submit", "values": {}}), &[]);
        let e = as_json(&p.expect("errors"));
        check!(p, e.contains(r#""t":"表に 2 件のエラーがあります""#), "errors = {e}");
        p.expect("cellErrors");
        // Edits sent right before a press are applied first.
        p.send(json!({"type": "cell", "field": "t", "row": 1, "col": 1, "value": "3"}), &[]);
        p.send(json!({"type": "cell", "field": "t", "row": 0, "col": 2, "value": "返品"}), &[]);
        p.send(json!({"type": "submit", "values": {}}), &[]);
        p.wait_closed();
    })];
    let src = r#"
const rows = [{ 名前: "a", 数: 1, 区分: "売上" }, { 名前: "b", 数: "x", 区分: "売上" }];
const v = await ui.form({ fields: [{
  name: "t", type: "table", rows, editable: true,
  columns: [{ key: "名前", editable: false }, { key: "数", min: 0, required: true }, { key: "区分", type: "select", options: ["売上", "返品"] }],
  validateRow: (row) => (row.名前 === "b" && !(row.数 >= 2) ? "2 件以上" : ""),
}]});
console.log(JSON.stringify(v.t));
"#;
    form_case("edit.js", src, "[{\"名前\":\"a\",\"数\":1,\"区分\":\"返品\"},{\"名前\":\"b\",\"数\":3,\"区分\":\"売上\"}]\n", pages);
}

#[test]
fn form_window_live_checks() {
    let pages: Vec<PageFn> = vec![Box::new(|p| {
        p.expect("init");
        let check = |field: &str, values: Value, want: &str| {
            p.send(json!({"type": "check", "field": field, "values": values}), &[]);
            let got = as_json(&p.expect("fieldError"));
            check!(p, got == want, "check {field} = {got}, want {want}");
        };
        check("n", json!({"n": "x", "code": ""}), r#"{"error":"数値を入力してください","field":"n","type":"fieldError"}"#);
        check("n", json!({"n": "50", "code": ""}), r#"{"error":"10 以下の値を入力してください","field":"n","type":"fieldError"}"#);
        check("n", json!({"n": "5", "code": ""}), r#"{"error":"","field":"n","type":"fieldError"}"#);
        check("code", json!({"n": "5", "code": "B1"}), r#"{"error":"5 で始めてください","field":"code","type":"fieldError"}"#);
        check("code", json!({"n": "5", "code": ""}), r#"{"error":"入力してください","field":"code","type":"fieldError"}"#);
        // The form's validate runs only when a button is pressed.
        check("code", json!({"n": "5", "code": "51"}), r#"{"error":"","field":"code","type":"fieldError"}"#);
        p.send(json!({"type": "submit", "values": {"n": "5", "code": "51"}}), &[]);
        let e = as_json(&p.expect("errors"));
        check!(p, e.contains(r#""error":"全体""#), "errors = {e}");
        p.send(json!({"type": "close"}), &[]);
        p.wait_closed();
    })];
    let src = r#"
await ui.form({
  fields: [
    { name: "n", type: "number", max: 10 },
    { name: "code", required: true, validate: (v, values) => (v.startsWith(String(values.n)) ? "" : values.n + " で始めてください") },
  ],
  validate: () => "全体",
});
"#;
    form_case("live.js", src, "", pages);
}

#[test]
fn form_window_on_change() {
    let dir = tempdir();
    for n in ["売上_202604.csv", "仕入_202604.csv"] {
        std::fs::write(dir.join(n), "").unwrap();
    }
    let sales = dir.join("売上_202604.csv").to_string_lossy().into_owned();
    let purchase = dir.join("仕入_202604.csv").to_string_lossy().into_owned();
    let pages: Vec<PageFn> = vec![Box::new(move |p| {
        let init = p.expect("init");
        let f = &init["fields"][0];
        check!(p, f["tooltip"] == "売上の CSV" && f["change"] == true, "field = {}", as_json(f));
        let f = &init["fields"][2];
        check!(p, f["value"] == "" && f["placeholder"] == "選んでください", "select = {}", as_json(f));
        let tips = as_json(&init["buttonTooltips"]);
        check!(p, tips == r#"["説明"]"#, "buttonTooltips = {tips}");
        // Typing (no change) does not run onChange.
        p.send(json!({"type": "check", "field": "sales", "values": {"sales": sales, "purchase": "", "kind": ""}}), &[]);
        p.expect("fieldError");
        // A change: onChange fills the other file (quiet, with its seq).
        p.send(json!({"type": "check", "field": "sales", "change": true, "values": {"sales": sales, "purchase": "", "kind": ""}}), &[]);
        p.expect("fieldError");
        let picked = p.expect("picked");
        check!(
            p,
            picked["field"] == "purchase" && picked["value"] == purchase.as_str() && picked["quiet"] == true && picked["seq"] == 1,
            "picked = {}",
            as_json(&picked)
        );
        // A submit sent before the page applied it keeps the value the script set.
        p.send(json!({"type": "submit", "seq": 0, "values": {"sales": sales, "purchase": "", "kind": "a"}}), &[]);
        p.wait_closed();
    })];
    let src = r#"
let changes = 0;
const v = await ui.form({
  fields: [
    { name: "sales", type: "file", tooltip: "売上の CSV", required: true, onChange(h, values, ctx) {
      changes++;
      const other = h.path.replace("売上_", "仕入_");
      if (!values.purchase && fs.exists(other)) ctx.setValue("purchase", other);
    } },
    { name: "purchase", type: "file", required: true },
    { name: "kind", type: "select", options: ["a", "b"], placeholder: "選んでください", required: true },
  ],
  buttons: [{ label: "?", tooltip: "説明", validate: false, onClick() {} }],
});
console.log(changes, v.purchase.name, v.kind);
"#;
    form_case("change.js", src, "1 仕入_202604.csv a\n", pages);
}

#[test]
fn chart_window() {
    let pages: Vec<PageFn> = vec![Box::new(|p| {
        let init = p.expect("init");
        let spec = as_json(&init["spec"]);
        check!(p, init["click"] == true && init["change"] == false && init["lang"] == "ja", "init = {}", as_json(&init));
        // Dates as { $date: ms }; functions are left out.
        check!(
            p,
            spec.contains(r#""日":{"$date":"#) && spec.contains(r#""type":"line""#) && !spec.contains("onClick"),
            "spec = {spec}"
        );
        check!(
            p,
            init["texts"]["today"] == "今日" && init["texts"]["views"]["Week"] == "週" && init["texts"]["planned"] == "予定 {0}%",
            "texts = {}",
            as_json(&init["texts"])
        );
        p.send(json!({"type": "click", "index": 1, "series": "売上"}), &[]);
        let u = p.expect("update");
        check!(p, u["spec"]["type"] == "column" && u["spec"]["data"].as_array().map(|a| a.len()) == Some(1), "update = {}", as_json(&u));
        p.wait_closed();
    })];
    let src = r#"
const rows = [{ 日: new Date(2026, 3, 1), 売上: 1 }, { 日: new Date(2026, 3, 2), 売上: 2 }];
const view = ui.chart({ type: "line", data: rows, onClick(row, info) {
  console.log("click", row.売上, info.index, info.series);
  view.update({ type: "column", data: rows.slice(0, 1) });
  view.close();
} });
await view.closed;
"#;
    form_case("chart.js", src, "click 2 1 売上\n", pages);
}

#[test]
fn form_window_chart() {
    let pages: Vec<PageFn> = vec![Box::new(|p| {
        let init = p.expect("init");
        let f = &init["fields"][1];
        check!(p, f["type"] == "chart" && f["chart"]["height"] == 250 && f["chart"]["change"] == true, "field = {}", as_json(f));
        check!(p, init["chartTexts"]["days"] == "{0}日", "chartTexts = {}", as_json(&init["chartTexts"]));
        // A task dragged by two days, then a click.
        let (s, e) = (1_794_268_800_000_i64, 1_794_700_800_000_i64);
        p.send(json!({"type": "chartChange", "field": "g", "index": 0, "changes": {"start": {"$date": s}, "end": {"$date": e}}}), &[]);
        p.send(json!({"type": "chartChange", "field": "g", "index": 0, "changes": {"progress": 0.5}}), &[]);
        p.send(json!({"type": "chartClick", "field": "g", "index": 0}), &[]);
        let err = p.expect("fieldError");
        check!(p, err["field"] == "g" && err["error"].as_str().unwrap_or("").contains("クリック"), "fieldError = {}", as_json(&err));
        p.send(json!({"type": "submit", "values": {"a": "x", "g": null}}), &[]);
        p.expect("running");
        let c = p.expect("chart");
        check!(p, c["field"] == "g" && c["spec"]["view"] == "week" && c["spec"]["type"] == "gantt", "chart = {}", as_json(&c));
        p.expect("done");
        p.send(json!({"type": "close"}), &[]);
        p.wait_closed();
    })];
    let src = r#"
const tasks = [{ 名前: "設計", 開始: new Date(2026, 10, 2), 終了: new Date(2026, 10, 6), 進捗: 0 }];
const v = await ui.form({
  fields: [
    { name: "a" },
    { name: "g", type: "chart", height: 10, chart: { type: "gantt", data: tasks, editable: true,
      onChange(row, changes) { console.log("change", Object.keys(changes).join(","), row === tasks[0]); },
      onClick() { throw new Error("クリックの失敗"); } } },
  ],
  onSubmit(values, ctx) { ctx.chart("g", { view: "week" }); },
});
console.log(v.a, v.g.length, v.g[0].開始.getTime() === 1794268800000, v.g[0].進捗);
"#;
    form_case("chartform.js", src, "change start,end true\nchange progress true\nx 1 true 0.5\n", pages);
}
