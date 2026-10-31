use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::testutil::{SharedBuf, lock_lang, repo_path, tempdir};
use crate::ui::{Mock, WindowOptions};

pub(crate) struct Res {
    pub code: i32,
    pub err: Option<Error>,
    pub stdout: String,
    pub stderr: String,
}

impl Res {
    fn err_text(&self) -> String {
        self.err.as_ref().map(|e| e.to_string()).unwrap_or_default()
    }
}

fn out(b: &SharedBuf) -> Out {
    Arc::new(Mutex::new(b.clone()))
}

/// Runs script with the mock UI in an empty temporary folder; with
/// debugger, in the debugger window (played by the fake page).
pub(crate) fn run_script_in(debugger: bool, script: &str, answers: &str, args: &[&str]) -> Res {
    let (o, e) = (SharedBuf::default(), SharedBuf::default());
    let m = Mock::new(answers, Some(e.sink())).unwrap();
    let dir = tempdir();
    let cfg = tempdir();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    unsafe { std::env::set_var("XLJS_CONFIG_DIR", &cfg) };
    let mut opts = Options {
        script_path: script.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        ui: Some(Arc::new(m)),
        stdout: Some(out(&o)),
        stderr: Some(out(&e)),
        version: "test".into(),
        ..Default::default()
    };
    if debugger {
        super::debug::tests::use_fake_page(&mut opts);
    }
    let r = run(opts);
    std::env::set_current_dir(prev).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cfg);
    let (code, err) = match r {
        Ok(c) => (c, None),
        Err(e) => (1, Some(e)),
    };
    Res { code, err, stdout: o.text(), stderr: e.text() }
}

pub(crate) fn run_script(script: &str, answers: &str, args: &[&str]) -> Res {
    run_script_in(false, script, answers, args)
}

fn write_script(name: &str, src: &str) -> String {
    let p = tempdir().join(name);
    std::fs::write(&p, src).unwrap();
    p.to_string_lossy().into_owned()
}

fn scripts() -> Vec<std::path::PathBuf> {
    let dir = repo_path("testdata/scripts");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.ends_with(".test.js") || n.ends_with(".test.ts")
        })
        .collect();
    v.sort();
    v
}

fn check_script_output(name: &str, r: &Res) -> Vec<String> {
    let fails = std::cell::RefCell::new(Vec::new());
    let want_in = |hay: &str, list: &[&str], what: &str| {
        for w in list {
            if !hay.contains(w) {
                fails.borrow_mut().push(format!("{name}: {what} does not contain {w:?}"));
            }
        }
    };
    match name {
        "app.test.js" => want_in(
            &r.stderr,
            &[
                r#"window.open("アプリのテスト", "https://app.xljs/ui/index.html", 640x480, titlebar)"#,
                r#"window.open("詳細", "https://app.xljs/ui/sub%20page.html", 300x200)"#,
                r#"window.closed("詳細")"#,
                r#"window.open("固定", "https://app.xljs/ui/sub%20page.html", 320x240, fixed)"#,
                r#"window.open("自前", "https://app.xljs/ui/sub%20page.html", 0x0, titlebar "fa-regular fa-calendar")"#,
                r#"window.open("OS", "https://app.xljs/ui/sub%20page.html", 0x0)"#,
            ],
            "mock log",
        ),
        "bridge.test.js" => {
            want_in(
                &r.stderr,
                &[
                    r#"window.init({"progress":0,"message":"準備中","when":{"$date":"#,
                    r##"window.$("#title").text("見出し")"##,
                    r##"window.$("#title").addClass("big")"##,
                    r##"window.$("#title").text() -> null"##,
                    r#"window.evaluate("(a) => a + 1") -> null"#,
                    "app: invoke add(1, 2) -> 3",
                    r#"-> {"year":2026,"file":"a.xlsx","dir":"directory","size":3}"#,
                    "app: invoke fail() -> error: RangeError: だめです",
                    r#"app: invoke missing() -> error: Error: 画面から呼ばれた関数 "missing" がありません"#,
                    r#"app: invoke notCallable() -> error: Error: 画面から呼ばれた関数 "notCallable""#,
                    "app: invoke dynamic(21) -> 42",
                    "app: invoke whoami() -> true",
                    "window.state.progress = 100",
                    r#"window.emit("done", {"ok":true})"#,
                    r#"window.invoke("render", [1,2]) -> 2"#,
                    "error: Error: ハンドラのエラー",
                ],
                "mock log",
            );
            for u in ["expected", "window.state.progress = 0", "window.state.after", "timeout"] {
                if r.stderr.contains(u) {
                    fails.borrow_mut().push(format!("{name}: mock log contains {u:?}"));
                }
            }
            want_in(&r.stdout, &["hello 画面\n", "state month 2026-04\n", "render -> 2\n"], "stdout");
        }
        "form.test.js" => want_in(
            &r.stderr,
            &[
                "form error: count: 10 以下の値を入力してください",
                "form error: day: 日付（YYYY-MM-DD）を入力してください",
                "form error: code: 英大文字 3 文字",
                "form error: inputs: ファイルが見つかりません",
                "form error: tax: 商品別は税込のみ",
                r#"progress("実行", total=3) start"#,
            ],
            "mock log",
        ),
        "formtable.test.js" => want_in(&r.stderr, &["form error: pick: 行を選択してください", r#"table("結果", 3 rows, ["顧客名" "合計"])"#], "mock log"),
        "table.test.js" => want_in(
            &r.stderr,
            &[
                r#"table("集計結果", 3 rows, ["顧客名" "件数" "合計" "日付" "備考"])"#,
                r#"table("列指定", 3 rows, ["顧客名" "金額"])"#,
                r#"table("配列", 2 rows, ["A" "B"])"#,
                r#"table("値", 2 rows, ["値"])"#,
            ],
            "mock log",
        ),
        "ui.test.js" => want_in(&r.stderr, &["showItemInFolder([", "a.xlsx", "b.csv", "openPath(", r#""https://example.com/""#], "mock log"),
        _ => {}
    }
    fails.into_inner()
}

/// Runs every testdata/scripts/*.test.{js,ts} with the mock UI; with
/// debugger, the way xljs --debug does.
pub(crate) fn test_scripts(debugger: bool) {
    let _l = lock_lang("ja");
    let mut fails = Vec::new();
    let filter = std::env::var("XLJS_TEST_SCRIPT").unwrap_or_default();
    for s in scripts() {
        let name = s.file_name().unwrap().to_string_lossy().into_owned();
        if !filter.is_empty() && !name.contains(&filter) {
            continue;
        }
        let answers = s.to_string_lossy().replace(".test.js", ".test.answers.json").replace(".test.ts", ".test.answers.json");
        let answers = if std::path::Path::new(&answers).exists() { answers } else { String::new() };
        let r = run_script_in(debugger, &s.to_string_lossy(), &answers, &[]);
        if std::env::var("XLJS_TEST_VERBOSE").is_ok() {
            eprintln!("{name}: {} {:?}", r.code, r.stdout.lines().last());
        }
        if r.err.is_some() || r.code != 0 {
            fails.push(format!("{name}: exit {}: {}\nstdout:\n{}\nstderr:\n{}", r.code, r.err_text(), r.stdout, r.stderr));
            continue;
        }
        if !r.stdout.contains(" ok") {
            fails.push(format!("{name}: missing completion marker in stdout:\n{}", r.stdout));
        }
        fails.extend(check_script_output(&name, &r));
    }
    assert!(fails.is_empty(), "{}", fails.join("\n\n"));
}

#[test]
fn scripts_pass() {
    test_scripts(false);
}

#[test]
fn entry_exports() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    std::fs::write(dir.join("lib.js"), "export function fromLib() {}\n").unwrap();
    let src = r#"export * from "./lib.js";
export async function f() {}
const z = 1;
export { z as "a b", z as default };
export let later = 0;
for (const name of ["f", "a b", "default", "later", "fromLib"]) {
  try { app.handle(name, () => 0); console.log("no error", name); } catch (e) {}
}
app.handle("other", () => 0);
console.log("done");
"#;
    let p = dir.join("main.js");
    std::fs::write(&p, src).unwrap();
    let r = run_script(&p.to_string_lossy(), "", &[]);
    assert!(r.code == 0 && r.err.is_none() && r.stdout == "done\n", "exit {} {} stdout {:?} stderr {:?}", r.code, r.err_text(), r.stdout, r.stderr);
}

#[test]
fn progress_cancel_ends_script() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    let answers = dir.join("answers.json");
    std::fs::write(&answers, r#"{"progress": [{"cancelAfter": 1}, {"cancelAfter": 1}]}"#).unwrap();
    for (name, src) in [
        ("sync", "for (const x of ui.withProgress([1, 2, 3])) console.log(x);\nconsole.log(\"saved\");\n"),
        ("async", "async function* g() { yield 1; yield 2; yield 3; }\nfor await (const x of ui.withProgress(g())) console.log(x);\nconsole.log(\"saved\");\n"),
    ] {
        let p = dir.join(format!("{name}.js"));
        std::fs::write(&p, src).unwrap();
        let r = run_script(&p.to_string_lossy(), &answers.to_string_lossy(), &[]);
        assert!(r.code == 1 && r.err.is_none() && !r.stdout.contains("saved"), "{name}: exit {} {} stdout {:?} stderr {:?}", r.code, r.err_text(), r.stdout, r.stderr);
    }
}

#[test]
fn error_stack_points_to_source() {
    let _l = lock_lang("ja");
    let r = run_script(&repo_path("testdata/scripts/_errors.js").to_string_lossy(), "", &[]);
    assert!(r.code == 1 && r.err.is_some(), "want failure, got {}", r.code);
    let msg = r.err_text();
    for want in ["RangeError: boom from lib", "_errlib.js:2:", "_errors.js:3:"] {
        assert!(msg.contains(want), "error {msg:?} does not contain {want:?}");
    }
    assert!(!msg.contains("prelude") && !msg.contains(BUNDLE_NAME), "internal frames leaked: {msg}");
}

#[test]
fn exit_codes() {
    let _l = lock_lang("ja");
    let cases: &[(&str, &str, i32, &str)] = &[
        ("exit", r#"try { exit(3) } catch { console.log("caught") }"#, 3, ""),
        ("exit-in-timer", r#"setTimeout(() => xljs.exit(4), 1); await sleep(100); console.log("no")"#, 4, ""),
        ("throw-after-await", r#"await sleep(1); throw new Error("late")"#, 1, "Error: late"),
        ("throw-value", r#"throw "plain""#, 1, "plain"),
        ("unhandled", r#"Promise.reject(new Error("lost")); await sleep(1)"#, 1, "処理されなかった Promise"),
        ("pending", r#"await new Promise(() => {})"#, 1, "完了しませんでした"),
        ("syntax", r#"const = 1"#, 1, "x.js:1:"),
        ("missing-import", r#"import "./nope.js""#, 1, "nope.js"),
    ];
    for (name, src, code, part) in cases {
        let r = run_script(&write_script("x.js", src), "", &[]);
        assert_eq!(r.code, *code, "{name}: err {} stdout {:?}", r.err_text(), r.stdout);
        if !part.is_empty() {
            assert!(r.err_text().contains(part), "{name}: err = {}, want {part:?}", r.err_text());
        }
        assert!(!r.stdout.contains("caught") && !r.stdout.contains("no"), "{name}: code after exit ran: {:?}", r.stdout);
    }
}

#[test]
fn args_and_console() {
    let _l = lock_lang("ja");
    let p = write_script("a.js", r#"console.log(xljs.args.join("|")); console.error("to stderr"); console.log("%s=%d", "n", 42)"#);
    let r = run_script(&p, "", &["x", "y z"]);
    assert_eq!(r.code, 0, "{}", r.err_text());
    assert_eq!(r.stdout, "x|y z\nn=42\n");
    assert!(r.stderr.contains("to stderr"));
}

#[test]
fn messages_follow_language() {
    let _l = lock_lang("ja");
    for (lang, want, want_fs, want_imp) in [
        ("ja", "のプロパティ 'name' は読み取れません", "見つかりません", "が見つかりません"),
        ("en", "Cannot read property 'name' of", "missing.csv", "Could not resolve"),
    ] {
        i18n::set_language(lang);
        let r = run_script(&write_script("x.js", "const o = null; console.log(o.name)"), "", &[]);
        assert!(r.err_text().contains(want), "{lang}: err = {}, want {want:?}", r.err_text());
        let r = run_script(&write_script("y.js", r#"fs.readText("nope/missing.csv")"#), "", &[]);
        assert!(r.err_text().contains(want_fs), "{lang}: fs err = {}", r.err_text());
        let r = run_script(&write_script("z.js", r#"import "./nope.js""#), "", &[]);
        assert!(r.err_text().contains(want_imp), "{lang}: import err = {}", r.err_text());
    }
}

#[test]
fn default_font_follows_language() {
    let _l = lock_lang("ja");
    for (lang, font) in [("ja", "游ゴシック"), ("en", "Calibri")] {
        i18n::set_language(lang);
        let src = format!("const f = workbooks.create().defaultFont.name; if (f !== \"{font}\") throw new Error(f)");
        let r = run_script(&write_script("f.js", &src), "", &[]);
        assert_eq!(r.code, 0, "{lang}: {}", r.err_text());
    }
}

#[test]
fn dayjs_locale_follows_language() {
    let _l = lock_lang("ja");
    for (lang, want) in [("ja", "ja 2026年4月1日(水)"), ("en", "en Wed, April 1, 2026")] {
        i18n::set_language(lang);
        let src = format!(
            "const s = dayjs.locale() + \" \" + dayjs(\"2026-04-01\").format(dayjs.locale() === \"ja\" ? \"YYYY年M月D日(ddd)\" : \"ddd, MMMM D, YYYY\")\nif (s !== \"{want}\") throw new Error(s)"
        );
        let r = run_script(&write_script("d.js", &src), "", &[]);
        assert_eq!(r.code, 0, "{lang}: {}", r.err_text());
    }
}

#[test]
fn app_mode() {
    let _l = lock_lang("ja");
    let src = "// ==xljs==\n// @mode app\n// @unknown 1\n// ==/xljs==\nconsole.log('ran')\n";
    let p = write_script("app.js", src);
    // The console UI cannot show windows: the script must not run.
    let (o, e) = (SharedBuf::default(), SharedBuf::default());
    let r = run(Options {
        script_path: p,
        ui: Some(Arc::new(crate::ui::Console::with_io(std::io::Cursor::new(Vec::new()), SharedBuf::default()))),
        stdout: Some(out(&o)),
        stderr: Some(out(&e)),
        ..Default::default()
    });
    let err = r.expect_err("console must fail");
    assert!(err.to_string().contains("ネイティブ UI") && o.text().is_empty(), "{err}");
    assert!(e.text().contains("@unknown"), "no warning for an unknown key: {:?}", e.text());
    // Bad metadata stops the script before it runs.
    let bad = write_script("bad.js", "// ==xljs==\n// @mode window\n// ==/xljs==\nconsole.log('ran')\n");
    let r = run_script(&bad, "", &[]);
    assert!(r.code == 1 && r.err_text().contains("bad.js:2"), "{}", r.err_text());
}

/// A mock whose windows stay open until closed by the script or until
/// after (when set) has passed, as if the user closed them.
pub(crate) struct SlowUi {
    pub m: Mock,
    pub after: Option<Duration>,
    pub events: Arc<Mutex<Vec<String>>>,
    pub windows: bool,
    pub icons: Arc<Mutex<Vec<usize>>>,
}

struct SlowWindow {
    o: WindowOptions,
    events: Arc<Mutex<Vec<String>>>,
    done: Mutex<bool>,
}

impl SlowWindow {
    fn close_as(&self, by: &str) {
        let mut d = self.done.lock().unwrap();
        if *d {
            return;
        }
        *d = true;
        drop(d);
        self.events.lock().unwrap().push(format!("closed by {by}"));
        if let Some(f) = &self.o.on_closed {
            f();
        }
    }
}

impl crate::ui::Window for SlowWindow {
    fn post(&self, _: String) {}
    fn close(&self) {
        self.close_as("script");
    }
}

impl SlowUi {
    pub fn new(after: Option<Duration>, windows: bool) -> SlowUi {
        SlowUi { m: Mock::from_json("", None).unwrap(), after, events: Default::default(), windows, icons: Default::default() }
    }
}

macro_rules! forward_ui {
    ($t:ty) => {
        impl crate::ui::Ui for $t {
            fn name(&self) -> String { self.m.name() }
            fn alert(&self, a: &str, b: &str) { self.m.alert(a, b) }
            fn confirm(&self, a: &str, b: &str) -> bool { self.m.confirm(a, b) }
            fn prompt(&self, a: &str, b: &str, c: &str) -> Option<String> { self.m.prompt(a, b, c) }
            fn open_files(&self, o: &crate::ui::PickerOptions) -> Vec<String> { self.m.open_files(o) }
            fn save_file(&self, o: &crate::ui::PickerOptions) -> Option<String> { self.m.save_file(o) }
            fn pick_directory(&self, o: &crate::ui::PickerOptions) -> Option<String> { self.m.pick_directory(o) }
            fn progress(&self, o: &crate::ui::ProgressOptions) -> Arc<dyn crate::ui::Progress> { self.m.progress(o) }
            fn show_error(&self, a: &str, b: &str) { self.m.show_error(a, b) }
            fn show_in_folder(&self, p: &[String]) -> crate::Result<()> { self.m.show_in_folder(p) }
            fn open_path(&self, t: &str) -> crate::Result<()> { self.m.open_path(t) }
            fn open_window(&self, o: WindowOptions) -> crate::Result<Arc<dyn crate::ui::Window>> { self.open(o) }
            fn supports_windows(&self) -> bool { self.windows }
            fn form(&self, o: &crate::ui::FormOptions) -> Option<serde_json::Map<String, serde_json::Value>> { self.m.form(o) }
            fn table(&self, o: &crate::ui::TableOptions) -> Option<Vec<i64>> { self.m.table(o) }
            fn notify(&self, a: &str, b: &str) { self.m.notify(a, b) }
        }
    };
}


impl SlowUi {
    fn open(&self, o: WindowOptions) -> crate::Result<Arc<dyn crate::ui::Window>> {
        self.icons.lock().unwrap().push(o.icon.len());
        self.events.lock().unwrap().push(format!("open {}", o.title));
        let w = Arc::new(SlowWindow { o, events: self.events.clone(), done: Mutex::new(false) });
        if let Some(after) = self.after {
            let w2 = w.clone();
            std::thread::spawn(move || {
                std::thread::sleep(after);
                w2.close_as("user");
            });
        }
        Ok(w)
    }
}

forward_ui!(SlowUi);

fn quiet() -> Out {
    Arc::new(Mutex::new(std::io::sink()))
}

#[test]
fn app_window_keeps_running() {
    let _l = lock_lang("ja");
    let app = "// ==xljs==\n// @mode app\n// ==/xljs==\n";
    // The script finishes at once, but the process waits for the window.
    let u = Arc::new(SlowUi::new(Some(Duration::from_millis(150)), false));
    let o = SharedBuf::default();
    let start = Instant::now();
    let p = write_script("a.js", &format!("{app}app.main.closed.then(() => console.log('closed')); console.log('end of script')"));
    let code = run(Options { script_path: p, ui: Some(u.clone()), stdout: Some(out(&o)), stderr: Some(quiet()), ..Default::default() }).unwrap();
    assert_eq!(code, 0);
    assert!(start.elapsed() >= Duration::from_millis(150) && o.text() == "end of script\nclosed\n", "{:?} {:?}", start.elapsed(), o.text());

    // exit() closes windows that are still open.
    let u = Arc::new(SlowUi::new(None, false));
    let p = write_script("b.js", &format!("{app}app.open({{ title: 'sub' }}); setTimeout(() => exit(5), 10)"));
    let code = run(Options { script_path: p, ui: Some(u.clone()), stdout: Some(quiet()), stderr: Some(quiet()), ..Default::default() }).unwrap();
    assert_eq!(code, 5);
    assert_eq!(u.events.lock().unwrap().join(","), "open b,open sub,closed by script,closed by script");

    // exit() after awaiting a window that the user closed.
    for src in [
        "await app.open({ title: 'sub' }).closed; exit(7); console.log('after')",
        "app.open({ title: 'sub' }).closed.then(() => { exit(7); console.log('after') })",
        "const r = await ui.table([1], { selectable: true }); if (!r.length) exit(7); console.log('after')",
    ] {
        let u = Arc::new(SlowUi::new(Some(Duration::from_millis(20)), true));
        let o = SharedBuf::default();
        let r = run(Options { script_path: write_script("d.js", src), ui: Some(u), stdout: Some(out(&o)), stderr: Some(quiet()), ..Default::default() });
        assert!(matches!(r, Ok(7)) && o.text().is_empty(), "{src}: {:?} stdout {:?}", r.map_err(|e| e.to_string()), o.text());
    }

    // Through ui::run the script runs on its own thread and the UI calls
    // (including the window callbacks) are served by the caller.
    let u = Arc::new(SlowUi::new(Some(Duration::from_millis(20)), false));
    let p = write_script("c.js", &format!("{app}await app.main.closed"));
    let r = crate::ui::run(u, move |su| run(Options { script_path: p, ui: Some(su), stdout: Some(quiet()), stderr: Some(quiet()), ..Default::default() }).map_err(|e| e.to_string()));
    assert_eq!(r, Ok(0));
}

#[test]
fn app_icon() {
    let _l = lock_lang("ja");
    let ico = std::fs::read(repo_path("assets/xljs.ico")).unwrap();
    let dir = tempdir();
    std::fs::write(dir.join("app.ico"), &ico).unwrap();
    std::fs::write(dir.join("bad.ico"), "not an icon").unwrap();
    let write = |name: &str, icon: &str| {
        let p = dir.join(name);
        std::fs::write(&p, format!("// ==xljs==\n// @mode app\n// @icon {icon}\n// ==/xljs==\nawait app.open({{ title: 'sub' }}).closed;\n")).unwrap();
        p.to_string_lossy().into_owned()
    };
    // The main window and the windows the script opens get the icon.
    let r = run_script(&write("ok.js", "app.ico"), "", &[]);
    assert!(r.code == 0 && r.err.is_none(), "{} {}", r.err_text(), r.stderr);
    assert_eq!(r.stderr.matches(&format!("window.icon({} bytes)", ico.len())).count(), 2, "{}", r.stderr);
    for (icon, want) in [("missing.ico", "見つかりません"), ("bad.ico", "アイコンとして読み込めません")] {
        let r = run_script(&write("e.js", icon), "", &[]);
        assert!(r.code == 1 && r.err_text().contains(want), "{icon}: {}", r.err_text());
    }
}

#[test]
fn form_icon() {
    let _l = lock_lang("ja");
    // @icon also applies to forms and tables of a script without @mode app.
    let ico = std::fs::read(repo_path("assets/xljs.ico")).unwrap();
    let dir = tempdir();
    std::fs::write(dir.join("app.ico"), &ico).unwrap();
    let p = dir.join("form.js");
    std::fs::write(&p, "// ==xljs==\n// @icon app.ico\n// ==/xljs==\nawait ui.form({ fields: [{ name: 'a' }] });\nawait ui.table([1]).closed;\n").unwrap();
    let u = Arc::new(SlowUi::new(Some(Duration::from_millis(20)), true));
    let cfg = tempdir();
    unsafe { std::env::set_var("XLJS_CONFIG_DIR", &cfg) };
    let code = run(Options { script_path: p.to_string_lossy().into_owned(), ui: Some(u.clone()), stdout: Some(quiet()), stderr: Some(quiet()), ..Default::default() });
    assert!(matches!(code, Ok(0)), "{:?}", code.map_err(|e| e.to_string()));
    assert_eq!(*u.icons.lock().unwrap(), vec![ico.len(), ico.len()]);
}

/// The code in the documents for AI chats (docs/ai) is valid syntax, since
/// the models copy it as is; prompt.md and the Copilot instructions stay
/// within the size limits of the chats' input boxes.
#[test]
fn ai_docs() {
    let re = regex::Regex::new("(?s)```js\n(.*?)```").unwrap();
    for (name, limit) in [
        ("prompt.md", 15000),
        ("reference.md", 0),
        ("copilot/instructions.md", 8000),
        ("copilot/skills/xljs-reference/SKILL.md", 0),
        ("copilot/skills/vba-port/SKILL.md", 0),
    ] {
        let src = std::fs::read_to_string(repo_path(&format!("docs/ai/{name}"))).unwrap().replace("\r\n", "\n");
        let n = src.chars().count();
        assert!(limit == 0 || n <= limit, "{name} is {n} characters; keep it within {limit}");
        let blocks: Vec<_> = re.captures_iter(&src).collect();
        assert!(!blocks.is_empty(), "{name}: no js code blocks");
        for (i, b) in blocks.iter().enumerate() {
            let p = write_script("block.js", &b[1]);
            if let Err(e) = bundle(&p) {
                panic!("{name}: code block {}: {e}", i + 1);
            }
        }
    }
    // The reference skill for Copilot agents is a copy of reference.md.
    let want = std::fs::read_to_string(repo_path("docs/ai/reference.md")).unwrap().replace("\r\n", "\n");
    let got = std::fs::read_to_string(repo_path("docs/ai/copilot/skills/xljs-reference/SKILL.md")).unwrap().replace("\r\n", "\n");
    assert_eq!(got, want, "copilot/skills/xljs-reference/SKILL.md differs from reference.md");
}

mod pages;

/// Runs src with the watchdog at limit and the confirm answers (on a
/// script thread of ui::run, like the CLI).
fn run_watched(src: &str, confirm: &str, limit: Duration) -> Res {
    let (o, e) = (SharedBuf::default(), SharedBuf::default());
    let m = Mock::from_json(&format!(r#"{{"confirm": {confirm}}}"#), Some(e.sink())).unwrap();
    let mut opts = Options {
        script_path: write_script("loop.js", src),
        stdout: Some(out(&o)),
        stderr: Some(out(&e)),
        watchdog: Some(limit),
        ..Default::default()
    };
    let r = crate::ui::run(Arc::new(m), move |su| {
        opts.ui = Some(su);
        run(opts)
    });
    let (code, err) = match r {
        Ok(c) => (c, None),
        Err(e) => (1, Some(e)),
    };
    Res { code, err, stdout: o.text(), stderr: e.text() }
}

#[test]
fn watchdog_stops_endless_loops() {
    let _l = lock_lang("ja");
    let limit = Duration::from_millis(200);
    let cases: &[(&str, &str)] = &[
        ("sync", "while (true) {}"),
        ("catch", r#"try { for (;;) {} } catch { console.log("caught") } finally { console.log("finally") }"#),
        ("calls", "function f() { return 1 } for (;;) f()"),
        ("after-await", "await sleep(1); while (true) {}"),
        ("timer", "setTimeout(() => { while (true) {} }, 1); await sleep(100000)"),
        ("promise-job", "Promise.resolve().then(() => { while (true) {} }); await sleep(100000)"),
    ];
    for (name, src) in cases {
        let start = Instant::now();
        let r = run_watched(src, "[false, true]", limit);
        assert_eq!(r.code, watchdog::STOPPED_CODE, "{name}: err {} stderr {:?}", r.err_text(), r.stderr);
        assert!(r.err.is_none() && r.stdout.is_empty(), "{name}: err {} stdout {:?}", r.err_text(), r.stdout);
        // "keep running" the first time, then "stop".
        assert_eq!(r.stderr.matches("confirm(").count(), 2, "{name}: {:?}", r.stderr);
        assert!(r.stderr.contains("無限ループ") && r.stderr.contains("スクリプトを中止しました"), "{name}: {:?}", r.stderr);
        assert!(start.elapsed() < Duration::from_secs(10), "{name}: took {:?}", start.elapsed());
    }
}

#[test]
fn watchdog_ignores_waiting_scripts() {
    let _l = lock_lang("ja");
    // Busy for less than the limit at a time, waiting in between: the
    // total time is far beyond the limit.
    let src = r#"
        const busy = (ms) => { const end = Date.now() + ms; while (Date.now() < end) {} };
        for (let i = 0; i < 8; i++) { busy(100); await sleep(20) }
        await new Promise((ok) => setTimeout(() => { busy(100); ok() }, 10));
        for (let i = 0; i < 8; i++) { busy(100); ui.confirm("q") }
        console.log("done");
    "#;
    let r = run_watched(src, "[false, false, false, false, false, false, false, false]", Duration::from_millis(400));
    assert!(r.code == 0 && r.err.is_none() && r.stdout.contains("done"), "exit {} {} {:?}", r.code, r.err_text(), r.stderr);
    assert!(!r.stderr.contains("無限ループ"), "asked: {:?}", r.stderr);
}

/// Runs src with the stop key read from down (on a script thread of
/// ui::run).
fn run_with_key(src: &str, confirm: &str, down: Arc<std::sync::atomic::AtomicBool>) -> Res {
    let (o, e) = (SharedBuf::default(), SharedBuf::default());
    let m = Mock::from_json(&format!(r#"{{"confirm": {confirm}}}"#), Some(e.sink())).unwrap();
    let mut key = watchdog::StopKey::parse("esc").unwrap();
    key.probe = Some(Arc::new(move |vk| vk == 0x1B && down.load(std::sync::atomic::Ordering::SeqCst)));
    let mut opts = Options {
        script_path: write_script("key.js", src),
        stdout: Some(out(&o)),
        stderr: Some(out(&e)),
        stop_key: Some(key),
        ..Default::default()
    };
    let r = crate::ui::run(Arc::new(m), move |su| {
        opts.ui = Some(su);
        run(opts)
    });
    let (code, err) = match r {
        Ok(c) => (c, None),
        Err(e) => (1, Some(e)),
    };
    Res { code, err, stdout: o.text(), stderr: e.text() }
}

/// Holds the key down from after until after + 100 ms (another thread).
fn press_later(down: &Arc<std::sync::atomic::AtomicBool>, after: Duration) {
    let d = down.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        d.store(true, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        d.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

#[test]
fn stop_key_stops_running_script() {
    let _l = lock_lang("ja");
    for (name, src) in [
        ("sync", "while (true) {}"),
        ("timer", "setTimeout(() => { for (;;) {} }, 1); await sleep(100000)"),
    ] {
        let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
        press_later(&down, Duration::from_millis(500));
        let r = run_with_key(src, "[true]", down);
        assert_eq!(r.code, watchdog::STOPPED_CODE, "{name}: err {} stderr {:?}", r.err_text(), r.stderr);
        assert!(r.stderr.contains("Esc キーが押されました") && r.stderr.contains("スクリプトを中止しました"), "{name}: {:?}", r.stderr);
    }
    // "keep running": the script goes on and ends normally.
    let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    press_later(&down, Duration::from_millis(400));
    let r = run_with_key("const end = Date.now() + 1200; while (Date.now() < end) {} console.log(\"done\")", "[false]", down);
    assert!(r.code == 0 && r.stdout.contains("done") && r.stderr.contains("Esc キー"), "exit {} {} {:?}", r.code, r.err_text(), r.stderr);
}

#[test]
fn stop_key_ignored_while_waiting() {
    let _l = lock_lang("ja");
    // Pressed while the script waits (a timer, as if Esc closed a
    // dialog): nothing is asked after it resumes.
    let down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    press_later(&down, Duration::from_millis(200));
    let src = "await sleep(600); const end = Date.now() + 500; while (Date.now() < end) {} console.log(\"done\")";
    let r = run_with_key(src, "[]", down);
    assert!(r.code == 0 && r.stdout.contains("done"), "exit {} {} {:?}", r.code, r.err_text(), r.stderr);
    assert!(!r.stderr.contains("confirm("), "asked: {:?}", r.stderr);
}
