use std::sync::{Arc, Mutex, mpsc};

use serde_json::{Value, json};

use super::project::Project;
use super::*;
use crate::testutil::{lock_lang, tempdir};
use crate::ui::{BrowserCommand, Mock, Window};

fn write(dir: &Path, rel: &str, text: &str) {
    write_text(&dir.join(rel), text).unwrap();
}

#[test]
fn download_matching() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    write(&dir, "集計.js", "// ==xljs==\n// @title 集計\n// ==/xljs==\nalert(1);\n");
    write(&dir, "lib.js", "export const a = 1;\n");
    write(&dir, "data.json", "{}");
    let p = Project::open(&dir.join("集計.js").to_string_lossy()).unwrap();
    assert_eq!(p.files(), vec!["集計.js", "data.json", "lib.js"]);
    let m = |name: &str, cur: &str| p.match_download(name, cur);
    assert_eq!(m("集計.js", ""), Some("集計.js".into()));
    assert_eq!(m("集計 (1).js", ""), Some("集計.js".into()));
    assert_eq!(m("集計_修正版.js", ""), Some("集計.js".into()));
    assert_eq!(m("集計.ts", ""), Some("集計.js".into()));
    assert_eq!(m("lib-fixed.js", ""), Some("lib.js".into()));
    assert_eq!(m("LIB.JS", ""), Some("lib.js".into()));
    // Another name: the script being edited, else the entry.
    assert_eq!(m("script.js", "lib.js"), Some("lib.js".into()));
    assert_eq!(m("script.js", ""), Some("集計.js".into()));
    assert_eq!(m("data_new.json", ""), Some("data.json".into()));
    assert_eq!(m("readme.txt", ""), None);

    // An app: its pages by name; no guessing for other names.
    let app = tempdir();
    write(&app, "app.js", "// ==xljs==\n// @mode app\n// @html ui/index.html\n// @assets ui/\n// ==/xljs==\n");
    write(&app, "ui/index.html", "<p>");
    write(&app, "ui/style.css", "p{}");
    write(&app, "ui/index-list.html", "<p>");
    write(&app, "ui/.hidden/x.html", "<p>");
    let p = Project::open(&app.join("app.js").to_string_lossy()).unwrap();
    assert_eq!(p.files(), vec!["app.js", "ui/index-list.html", "ui/index.html", "ui/style.css"]);
    assert_eq!(p.match_download("index_修正.html", ""), Some("ui/index.html".into()));
    assert_eq!(p.match_download("index-list (2).html", ""), Some("ui/index-list.html".into()));
    assert_eq!(p.match_download("style (1).css", ""), Some("ui/style.css".into()));
    assert_eq!(p.match_download("app_v2.js", ""), Some("app.js".into()));
    assert_eq!(p.match_download("main.js", ""), None);
    assert_eq!(p.match_download("other.html", ""), None);

    assert!(p.path("../x.js").is_err() && p.path("C:/x.js").is_err() && p.path("/x").is_err() && p.path("").is_err());
    assert!(p.path("ui/index.html").is_ok());
}

#[test]
fn versions_keep_and_restore() {
    let dir = tempdir();
    write(&dir, "a.js", "v1");
    assert!(versions::list(&dir, "a.js").is_empty());
    assert_eq!(versions::replace(&dir, "a.js", "v2", "first").unwrap(), Some(1));
    assert_eq!(versions::replace(&dir, "a.js", "v3", "second").unwrap(), Some(2));
    // The same content as the newest version is not kept twice.
    assert_eq!(versions::keep(&dir, "a.js", "x").unwrap(), Some(3));
    assert_eq!(versions::keep(&dir, "a.js", "x").unwrap(), None);
    let l = versions::list(&dir, "a.js");
    assert_eq!(l.iter().map(|v| (v.n, v.note.as_str())).collect::<Vec<_>>(), vec![(1, "first"), (2, "second"), (3, "x")]);
    assert_eq!(versions::read(&dir, "a.js", 1).unwrap(), "v1");
    assert!(dir.join(".xljs/versions/a.js/0001.js").is_file());
    assert_eq!(versions::restore(&dir, "a.js", 1, "back").unwrap(), "v1");
    assert_eq!(std::fs::read_to_string(dir.join("a.js")).unwrap(), "v1");
    // v3 was already kept as #3.
    assert_eq!(versions::list(&dir, "a.js").len(), 3);
    write(&dir, "ui/p.html", "<a>");
    versions::replace(&dir, "ui/p.html", "<b>", "n").unwrap();
    assert!(dir.join(".xljs/versions/ui/p.html/0001.html").is_file());
    // A new file has nothing to keep.
    assert_eq!(versions::replace(&dir, "new.js", "x", "n").unwrap(), None);
    assert_eq!(std::fs::read_to_string(dir.join("new.js")).unwrap(), "x");
}

#[test]
fn error_reports() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    let src: String = (1..=30).map(|i| format!("line{i}\n")).collect();
    write(&dir, "main.js", &src);
    write(&dir, "lib/util.js", "x\n");
    let abs = dir.join("lib").join("util.js").to_string_lossy().into_owned();
    let stack = format!("console line\nTypeError: x is not a function\n    at f (node_modules/a.js:1:1)\n    at g ({abs}:1:5)\n    at <anonymous> (main.js:12:3)\n");
    let loc = report::find_location(&stack, &dir).unwrap();
    assert_eq!(loc, report::Location { file: "lib/util.js".into(), line: 1, column: 5 });
    assert_eq!(report::last_error(&stack), "TypeError: x is not a function\n    at f (node_modules/a.js:1:1)\n    at g (".to_string() + &abs + ":1:5)\n    at <anonymous> (main.js:12:3)");
    // Syntax errors.
    let syn = "構文エラー:\nmain.js:3:7: Unexpected token\n    line3\n";
    assert_eq!(report::last_error(syn), "構文エラー:\nmain.js:3:7: Unexpected token\n    line3");
    assert_eq!(report::find_location(syn, &dir), Some(report::Location { file: "main.js".into(), line: 3, column: 7 }));
    // A stopped script.
    let stopped = "out\nスクリプトを中止しました\n中止した場所:\n    at spin (main.js:20:9)\n";
    assert_eq!(report::last_error(stopped), "中止した場所:\n    at spin (main.js:20:9)");
    assert_eq!(report::find_location(stopped, &dir).unwrap().line, 20);
    assert!(report::find_location("Error: no place", &dir).is_none());

    let ex = report::excerpt(&src, 12, 2);
    assert_eq!(ex, " 10| line10\n 11| line11\n>12| line12\n 13| line13\n 14| line14\n");
    assert_eq!(report::excerpt(&src, 1, 1), ">1| line1\n 2| line2\n");

    let p = Project::open(&dir.join("main.js").to_string_lossy()).unwrap();
    let loc = report::Location { file: "main.js".into(), line: 12, column: 3 };
    let pr = report::prompt(&p, false, "TypeError: x is not a function", Some(&loc));
    assert!(pr.contains("「main.js」") && pr.contains("TypeError: x is not a function") && pr.contains(">12| line12") && pr.contains("12 行目"), "{pr}");
    let pr = report::prompt(&p, true, "", Some(&loc));
    assert!(pr.contains("中止") && !pr.contains("エラー:"), "{pr}");
    let intro = report::intro(Some(&p));
    assert!(intro.starts_with("あなたは「xljs」") && intro.contains("line30"), "{}", &intro[intro.len() - 200..]);
}

/// A window that records what the session does.
#[derive(Default)]
struct Rec {
    posts: Mutex<Vec<Value>>,
    cmds: Mutex<Vec<BrowserCommand>>,
}

impl Window for Rec {
    fn post(&self, msg: String) {
        self.posts.lock().unwrap().push(serde_json::from_str(&msg).unwrap());
    }
    fn close(&self) {}
    fn browser(&self, cmd: BrowserCommand) {
        self.cmds.lock().unwrap().push(cmd);
    }
}

impl Rec {
    /// The events of a type posted since the last call.
    fn take(&self, ty: &str) -> Vec<Value> {
        let mut p = self.posts.lock().unwrap();
        let out = p.iter().filter(|v| v["type"] == ty).cloned().collect();
        p.clear();
        out
    }
}

fn session(dir: &Path, answers: &str) -> (Session, Arc<Rec>, mpsc::Receiver<Event>) {
    let w = Arc::new(Rec::default());
    let u = Arc::new(Mock::from_json(answers, None).unwrap());
    let (tx, rx) = mpsc::channel();
    let o = DevOptions { path: String::new(), exe: "xljs".into(), run_args: vec![], settings: dir.join("settings.json") };
    (Session::new(u, w.clone(), o, tx, Arc::new(Mutex::new(None))), w, rx)
}

fn page(s: &mut Session, v: Value) {
    assert!(s.event(Event::Page(v.to_string(), vec![])));
}

#[test]
fn session_flow() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    write(&dir, "main.js", "alert(1);\n");
    let main = dir.join("main.js").to_string_lossy().into_owned();
    let (mut s, w, _rx) = session(&dir, "{}");
    page(&mut s, json!({"op": "ready"}));
    let init = w.take("init");
    assert_eq!(init[0]["texts"]["run"], "実行");
    assert!(init[0]["services"].as_array().unwrap().len() >= 3);

    page(&mut s, json!({"op": "open", "path": main}));
    let p = w.take("project");
    assert_eq!(p[0]["entry"], "main.js");
    assert_eq!(p[0]["text"], "alert(1);\n");
    assert_eq!(p[0]["chat"], "");
    assert_eq!(w.cmds.lock().unwrap().last(), Some(&BrowserCommand::Hide));

    // The chat used is remembered for the script and opened with it.
    s.event(Event::Browser(BrowserEvent::Url("https://chat.example/c/1".into())));
    assert_eq!(w.take("url")[0]["url"], "https://chat.example/c/1");
    page(&mut s, json!({"op": "open", "path": main}));
    assert_eq!(w.take("project")[0]["chat"], "https://chat.example/c/1");
    assert_eq!(w.cmds.lock().unwrap().last(), Some(&BrowserCommand::Navigate("https://chat.example/c/1".into())));
    // ...and in the recent scripts.
    page(&mut s, json!({"op": "ready"}));
    assert_eq!(w.take("init")[0]["recent"][0]["name"], "main.js");

    // A download: compared with the file it replaces.
    let dl = dir.join("dl");
    write(&dl, "main_fixed.js", "alert(2);\n");
    s.event(Event::Browser(BrowserEvent::Downloaded { path: dl.join("main_fixed.js").to_string_lossy().into(), name: "main_fixed.js".into() }));
    let d = w.take("download");
    assert_eq!((d[0]["target"].as_str(), d[0]["old"].as_str(), d[0]["text"].as_str()), (Some("main.js"), Some("alert(1);\n"), Some("alert(2);\n")));
    assert!(!dl.join("main_fixed.js").exists());

    // Accepting it: the old content becomes version 1.
    page(&mut s, json!({"op": "apply", "file": "main.js", "text": "alert(2);\n", "note": "main_fixed.js"}));
    let posts = w.posts.lock().unwrap().clone();
    w.posts.lock().unwrap().clear();
    let v = posts.iter().find(|v| v["type"] == "versions").unwrap();
    assert_eq!(v["versions"][0]["n"], 1);
    assert!(v["versions"][0]["note"].as_str().unwrap().contains("main_fixed.js"));
    assert_eq!(std::fs::read_to_string(dir.join("main.js")).unwrap(), "alert(2);\n");

    // Saving keeps a version once per session.
    page(&mut s, json!({"op": "save", "file": "main.js", "text": "alert(3);\n"}));
    page(&mut s, json!({"op": "save", "file": "main.js", "text": "alert(4);\n"}));
    assert_eq!(versions::list(&dir, "main.js").len(), 2);

    // Back to version 1.
    page(&mut s, json!({"op": "restore", "file": "main.js", "n": 1}));
    assert_eq!(w.take("applied")[0]["text"], "alert(1);\n");
    assert_eq!(std::fs::read_to_string(dir.join("main.js")).unwrap(), "alert(1);\n");

    // Files outside the folder are refused.
    page(&mut s, json!({"op": "read", "file": "../x.js"}));
    assert!(w.take("error")[0]["message"].as_str().unwrap().contains("../x.js"));

    // The end of a failed run: where, and a request for the AI.
    s.running = Some(Running { run: fake_run(&dir), kind: RunKind::Run, stderr: String::new(), preview: None });
    let id = s.running.as_ref().unwrap().run.id;
    s.event(Event::Run(RunEvent::Output { id, stderr: true, text: "ReferenceError: foo is not defined\n    at <anonymous> (main.js:1:1)\n".into() }));
    s.event(Event::Run(RunEvent::Exited { id, code: Some(1) }));
    let e = w.take("runEnded");
    assert_eq!(e[0]["location"], json!({"file": "main.js", "line": 1, "column": 1}));
    assert!(e[0]["prompt"].as_str().unwrap().contains("ReferenceError"), "{}", e[0]);

    assert!(!s.event(Event::Closed));
}

#[test]
fn chat_follows_the_script() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    write(&dir, "a.js", "1");
    write(&dir, "b.js", "2");
    let (mut s, w, _rx) = session(&dir, "{}");
    page(&mut s, json!({"op": "ready"}));
    // A chat opened before a script: the script opened next takes it.
    page(&mut s, json!({"op": "chat", "url": "https://chat.example/"}));
    s.event(Event::Browser(BrowserEvent::Url("https://chat.example/c/9".into())));
    page(&mut s, json!({"op": "open", "path": dir.join("a.js").to_string_lossy()}));
    assert_eq!(w.take("project")[0]["chat"], "https://chat.example/c/9");
    // Back to the start: the script forgets it.
    page(&mut s, json!({"op": "chat", "url": ""}));
    assert_eq!(w.cmds.lock().unwrap().last(), Some(&BrowserCommand::Hide));
    s.event(Event::Browser(BrowserEvent::Url("about:blank".into())));
    page(&mut s, json!({"op": "open", "path": dir.join("a.js").to_string_lossy()}));
    assert_eq!(w.take("project")[0]["chat"], "");
}

#[test]
fn dropped_files_open_the_script() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    write(&dir, "a.js", "1");
    write(&dir, "data.csv", "x");
    let path = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let (mut s, w, _rx) = session(&dir, "{}");
    page(&mut s, json!({"op": "ready"}));
    let drop = |s: &mut Session, files: Vec<String>| assert!(s.event(Event::Page(json!({"op": "drop"}).to_string(), files)));
    // The first script among the files (paths not available are "").
    drop(&mut s, vec![String::new(), path("data.csv"), path("a.js")]);
    assert_eq!(w.take("project")[0]["entry"], "a.js");
    // Not a script: an error, and the script stays open.
    drop(&mut s, vec![path("data.csv")]);
    assert!(w.take("error")[0]["message"].as_str().unwrap().contains("data.csv"));
    assert_eq!(s.current, "a.js");
    drop(&mut s, vec![]);
    assert!(w.take("error").is_empty());
}

/// A finished run (a process that ends at once).
fn fake_run(dir: &Path) -> Run {
    let (tx, _rx) = mpsc::channel::<RunEvent>();
    let exe = if cfg!(windows) { "cmd" } else { "true" };
    let args: Vec<String> = if cfg!(windows) { vec!["/c".into(), "exit".into()] } else { vec![] };
    Run::start(Path::new(exe), &args, dir, &[], 77, tx, |e| e).unwrap()
}

#[test]
fn runner_reports_output_and_exit() {
    if cfg!(windows) {
        return;
    }
    let dir = tempdir();
    let (tx, rx) = mpsc::channel::<RunEvent>();
    let args = vec!["-c".to_string(), "printf 'あいう'; printf 'err' >&2; exit 3".to_string()];
    let _r = Run::start(Path::new("sh"), &args, &dir, &[("X".into(), "1".into())], 5, tx, |e| e).unwrap();
    let (mut out, mut err, mut code) = (String::new(), String::new(), None);
    for e in rx {
        match e {
            RunEvent::Output { stderr: false, text, .. } => out.push_str(&text),
            RunEvent::Output { stderr: true, text, .. } => err.push_str(&text),
            RunEvent::Exited { id, code: c } => {
                assert_eq!(id, 5);
                code = c;
                break;
            }
        }
    }
    assert_eq!((out.as_str(), err.as_str(), code), ("あいう", "err", Some(3)));
}

fn port_answers(book: &str) -> wizard::Answers {
    wizard::Answers {
        kind: "port".into(),
        books: vec![book.into()],
        include_vba: true,
        include_sheets: true,
        sample_rows: 2,
        input: "yes".into(),
        input_note: "販売システムの売上 xlsx".into(),
        output: "auto".into(),
        form: "yes".into(),
        mode: "auto".into(),
        name_mode: "auto".into(),
        intro: false,
        ..Default::default()
    }
}

#[test]
fn wizard_prompts() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    // A workbook with data and formulas copied down.
    let book = dir.join("売上.xlsx");
    let mut wb = umya_spreadsheet::new_file();
    {
        let ws = wb.sheet_by_name_mut("Sheet1").unwrap();
        ws.set_name("明細");
        for (c, h) in ["日付", "顧客", "数量", "単価", "金額"].iter().enumerate() {
            ws.cell_mut((c as u32 + 1, 1)).set_value(*h);
        }
        for r in 2..=6u32 {
            ws.cell_mut((2, r)).set_value(format!("顧客{r}"));
            ws.cell_mut((3, r)).set_value_number(r as f64);
            ws.cell_mut((4, r)).set_value_number(100.0);
            ws.cell_mut((5, r)).set_formula(format!("C{r}*D{r}"));
        }
        ws.cell_mut((5, 7)).set_formula("SUM(E2:E6)");
    }
    umya_spreadsheet::writer::xlsx::write(&wb, &book).unwrap();
    let o = crate::api::workbook_outline(&book.to_string_lossy(), 2).unwrap();
    assert!(o.contains("シート「明細」（A1:E7、7 行 × 5 列）"), "{o}");
    assert!(o.contains("先頭 2 行") && o.contains("| 1 | 日付 | 顧客 |") && !o.contains("顧客3"), "{o}");
    assert!(o.contains("E2: `=C2*D2` （E2〜E6 に同じ形の数式が 5 セル）") && o.contains("E7: `=SUM(E2:E6)`"), "{o}");
    assert!(!crate::api::workbook_outline(&book.to_string_lossy(), 0).unwrap().contains("先頭"));

    // A port: the macro, the sheets and the conditions.
    let macro_book = crate::testutil::repo_path("testdata/scripts/macro.xlsm").to_string_lossy().into_owned();
    let a = port_answers(&macro_book);
    let p = wizard::prompt(&a);
    assert!(p.starts_with("# 依頼: Excel のマクロ（VBA）を xljs に移植する"), "{p}");
    assert!(p.contains("Sub 売上集計()") && p.contains("## ブックのシート構成"), "{p}");
    assert!(p.contains("- 入力: 入力フォーム（ui.form）") && p.contains("- 入力ファイル: あり — 販売システムの売上 xlsx"), "{p}");
    assert!(p.contains("内容が分かる短い日本語の名前") && p.contains("特になし。マクロと同じ動作"), "{p}");
    assert!(p.contains("まずマクロが何をしているか") && !p.contains("あなたは「xljs」"), "{p}");
    assert_eq!(wizard::file_name(&a), "新しいスクリプト.js");
    // Without the code and with the instructions first.
    let p = wizard::prompt(&wizard::Answers { include_vba: false, include_sheets: false, intro: true, ..a.clone() });
    assert!(p.starts_with("あなたは「xljs」") && !p.contains("Sub 売上集計()") && !p.contains("シート構成"));

    // A new app with a name: no form question, the page asked for.
    let a = wizard::Answers {
        kind: "new".into(),
        mode: "app".into(),
        form: "yes".into(),
        name_mode: "name".into(),
        name: "在庫/管理".into(),
        overview: "在庫を一覧で見たい".into(),
        ..Default::default()
    };
    let p = wizard::prompt(&a);
    assert_eq!(wizard::file_name(&a), "在庫_管理.js");
    assert!(p.contains("「在庫_管理.js」") && p.contains("## やりたいこと\n\n在庫を一覧で見たい") && p.contains("ui/index.html"), "{p}");
    assert!(!p.contains("- 入力:") && !p.contains("入力ファイル"), "{p}");
    assert_eq!(wizard::file_name(&wizard::Answers { name_mode: "name".into(), name: "a.ts".into(), ..Default::default() }), "a.ts");
}

#[test]
fn wizard_creates_and_ai_names() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    let (mut s, w, _rx) = session(&dir, "{}");
    page(&mut s, json!({"op": "ready"}));
    let a = json!({"kind": "new", "mode": "auto", "nameMode": "auto", "overview": "x"});
    page(&mut s, json!({"op": "wizardPrompt", "answers": a}));
    assert!(w.take("wizardPrompt")[0]["text"].as_str().unwrap().contains("## やりたいこと\n\nx"));
    page(&mut s, json!({"op": "wizardCreate", "answers": a, "path": dir.join("nope").to_string_lossy(), "text": "T"}));
    assert!(!w.take("error").is_empty());
    page(&mut s, json!({"op": "wizardCreate", "answers": a, "path": dir.to_string_lossy(), "text": "T"}));
    let posts = w.posts.lock().unwrap().clone();
    w.posts.lock().unwrap().clear();
    assert!(posts.iter().any(|v| v["type"] == "project" && v["entry"] == "新しいスクリプト.js"));
    assert!(posts.iter().any(|v| v["type"] == "copy" && v["text"] == "T"));
    assert!(read_text(&dir.join("新しいスクリプト.js")).unwrap().contains("AI の回答で置き換えます"));
    s.event(Event::Browser(BrowserEvent::Url("https://chat.example/c/2".into())));

    // The AI's file names the script.
    let dl = dir.join("dl");
    write(&dl, "売上集計 (1).js", "alert(1);\n");
    s.event(Event::Browser(BrowserEvent::Downloaded { path: dl.join("売上集計 (1).js").to_string_lossy().into(), name: "売上集計 (1).js".into() }));
    let d = w.take("download");
    assert_eq!((d[0]["target"].as_str(), d[0]["rename"].as_str()), (Some("新しいスクリプト.js"), Some("売上集計.js")));
    page(&mut s, json!({"op": "apply", "file": "新しいスクリプト.js", "text": "alert(1);\n", "note": "売上集計 (1).js", "rename": "売上集計.js"}));
    let p = w.take("project");
    assert_eq!((p[0]["entry"].as_str(), p[0]["text"].as_str(), p[0]["chat"].as_str()), (Some("売上集計.js"), Some("alert(1);\n"), Some("https://chat.example/c/2")));
    assert!(!dir.join("新しいスクリプト.js").exists());
    assert_eq!(versions::list(&dir, "売上集計.js").len(), 1);
    // Named now: the next answer does not rename it.
    write(&dl, "売上集計_v2.js", "alert(2);\n");
    s.event(Event::Browser(BrowserEvent::Downloaded { path: dl.join("売上集計_v2.js").to_string_lossy().into(), name: "売上集計_v2.js".into() }));
    assert_eq!(w.take("download")[0]["rename"], Value::Null);
}
