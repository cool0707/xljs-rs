use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use super::console_form::{parse_selection, render_table};
use super::*;
use crate::testutil::{SharedBuf, lock_lang};

fn console(input: &str, out: &SharedBuf) -> Console {
    Console::with_io(Cursor::new(input.as_bytes().to_vec()), out.clone())
}

fn field(name: &str, label: &str, kind: &str) -> FormField {
    FormField { name: name.into(), label: label.into(), kind: kind.into(), ..Default::default() }
}

fn opt(v: &str) -> FormOption {
    FormOption { value: v.into(), label: v.into() }
}

#[test]
fn console_form() {
    let _l = lock_lang("ja");
    let out = SharedBuf::default();
    let input = ["", "9", "商品別", "y", r#""C:\data""#, "a.xlsx; b.xlsx", ""].join("\n");
    let c = console(&input, &out);
    let o = FormOptions {
        title: "設定".into(),
        error: "全体のエラー".into(),
        fields: vec![
            FormField { value: json!("太郎"), required: true, error: "短すぎます".into(), ..field("name", "名前", "text") },
            FormField { value: json!("顧客別"), options: vec![opt("顧客別"), opt("商品別")], ..field("mode", "単位", "select") },
            FormField { value: json!(false), ..field("tax", "税込", "checkbox") },
            field("src", "フォルダ", "folder"),
            FormField { accept: vec![".xlsx".into()], ..field("in", "入力", "files") },
        ],
        ..Default::default()
    };
    let v = c.form(&o).unwrap_or_else(|| panic!("cancelled:\n{}", out.text()));
    assert_eq!(Value::Object(v), json!({"name": "太郎", "mode": "商品別", "tax": true, "src": r"C:\data", "in": ["a.xlsx", "b.xlsx"]}));
    for s in ["[設定]", "! 全体のエラー", "! 名前: 短すぎます", "名前 * [太郎]: ", " *1) 顧客別", "1 から 2 までの番号", "税込 [y/N]", "(*.xlsx)"] {
        assert!(out.text().contains(s), "output does not contain {s:?}:\n{}", out.text());
    }
    // End of input cancels.
    let c = console("", &out);
    assert!(c.form(&FormOptions { fields: vec![field("a", "", "text")], ..Default::default() }).is_none());
}

#[test]
fn console_form_tables_and_tabs() {
    let _l = lock_lang("ja");
    let out = SharedBuf::default();
    let c = console(&["", "5", "2-3", "", "-"].join("\n"), &out);
    let rows: Vec<Vec<String>> = [["A社", "2"], ["B社", "1"], ["C社", "5"]].iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
    let cols = vec!["顧客".to_string(), "件数".to_string()];
    let table = |name: &str, label: &str, select: &str, value: Value, tab: &str| FormField {
        columns: cols.clone(),
        rows: rows.clone(),
        select: select.into(),
        value,
        tab: tab.into(),
        ..field(name, label, "table")
    };
    let o = FormOptions {
        fields: vec![
            FormField { value: json!("x"), group: "グループ".into(), group_help: "まとめた項目".into(), ..field("a", "共通", "text") },
            FormField { columns: vec!["名前".into()], rows: vec![vec!["表示だけ".into()]], tab: "基本".into(), ..field("info", "一覧", "table") },
            table("pick", "対象", SELECT_MULTIPLE, json!([]), "基本"),
            table("one", "一つ", SELECT_SINGLE, json!([0.0]), "詳細"),
            table("none", "なし", SELECT_MULTIPLE, json!([1.0]), "詳細"),
        ],
        ..Default::default()
    };
    let v = c.form(&o).unwrap_or_else(|| panic!("cancelled:\n{}", out.text()));
    assert_eq!(Value::Object(v), json!({"a": "x", "pick": [1, 2], "one": [0], "none": []}));
    for s in ["-- グループ --\nまとめた項目\n", "== 基本 ==", "== 詳細 ==", "表示だけ", "│ 3 │ C社", "Enter で 1 のまま", "Enter で選択なし", "1 から 3 までの番号"] {
        assert!(out.text().contains(s), "output does not contain {s:?}:\n{}", out.text());
    }
}

#[test]
fn console_form_buttons() {
    let _l = lock_lang("ja");
    let out = SharedBuf::default();
    let c = console("\n\n\n2\n\n4\n読込\n", &out);
    let o = FormOptions {
        fields: vec![FormField { name: "a".into(), value: json!("x"), ..Default::default() }],
        submit: "OK".into(),
        buttons: vec!["読込".into(), "確認".into()],
        ..Default::default()
    };
    for want in [Value::Null, json!(1), json!(0)] {
        let v = c.form(&o).unwrap();
        assert!(v["$button"] == want && v["a"] == "x", "{v:?} want {want}\n{}", out.text());
    }
    assert!(out.text().contains("実行する操作") && out.text().contains(" *3) OK"), "{}", out.text());
}

#[test]
fn console_form_editable_table() {
    let _l = lock_lang("ja");
    let out = SharedBuf::default();
    let c = console("9\n2\n200\n1\ny\n\n", &out);
    let o = FormOptions {
        fields: vec![FormField {
            columns: ["名前", "金額", "区分", "済"].iter().map(|s| s.to_string()).collect(),
            rows: vec![vec!["a".into(), "1".into(), "売上".into(), "false".into()], vec!["b".into(), "x".into()]],
            editable: vec![
                FormCellColumn { index: 1, kind: "number".into(), ..Default::default() },
                FormCellColumn { index: 2, kind: "select".into(), options: vec![opt("売上"), opt("返品")] },
                FormCellColumn { index: 3, kind: "checkbox".into(), ..Default::default() },
            ],
            cell_errors: vec![
                FormCellError { row: 1, col: 1, message: "数値を入力してください".into() },
                FormCellError { row: 1, col: -1, message: "確認".into() },
            ],
            ..field("t", "", "table")
        }],
        ..Default::default()
    };
    let v = c.form(&o).unwrap();
    assert_eq!(v["$cells"], json!({"t": [[1, 1, "200"], [1, 2, "売上"], [1, 3, true]]}), "{}", out.text());
    assert!(!v.contains_key("t"));
    for s in ["! 2 行目 金額: 数値を入力してください", "! 2 行目: 確認", "1 から 2 までの番号"] {
        assert!(out.text().contains(s), "output does not contain {s:?}:\n{}", out.text());
    }
}

#[test]
fn console_table() {
    let _l = lock_lang("ja");
    let out = SharedBuf::default();
    let c = console("4\n3-2\n1, 3-4\n", &out);
    let rows: Vec<Vec<String>> = vec![vec!["あい".into(), "1".into()], vec!["b".into(), "22".into()], vec!["c\nd".into(), "".into()], vec!["e".into()]];
    let mut o = TableOptions { title: "表".into(), columns: vec!["名前".into(), "数".into()], rows, select: SELECT_MULTIPLE.into() };
    assert_eq!(c.table(&o), Some(vec![3]));
    assert_eq!(c.table(&o), Some(vec![0, 2, 3]));
    let want = "┌───┬──────┬────┐\n│ # │ 名前 │ 数 │\n├───┼──────┼────┤\n│ 1 │ あい │ 1  │\n│ 2 │ b    │ 22 │\n│ 3 │ c d  │    │\n│ 4 │ e    │    │\n└───┴──────┴────┘\n";
    assert_eq!(render_table(&o.columns, &o.rows), want);
    let c = console("\n", &out);
    assert!(c.table(&o).is_none(), "empty input must cancel");
    o.select = SELECT_NONE.into();
    assert!(c.table(&o).is_some(), "display only");
}

#[test]
fn selection() {
    for (input, want) in [("1", vec![0]), ("2,1", vec![1, 0]), ("1-3", vec![0, 1, 2]), ("1 2、2", vec![0, 1])] {
        assert_eq!(parse_selection(input, 5), Some(want), "{input}");
    }
    for input in ["0", "6", "a", "3-1", "1-9", ","] {
        assert!(parse_selection(input, 5).is_none(), "{input} accepted");
    }
}

/// Builds an .ico file with one fake image per size; each image's data is
/// its size.
fn make_ico(sizes: &[usize]) -> Vec<u8> {
    let mut head = Vec::new();
    for v in [0u16, 1, sizes.len() as u16] {
        head.extend_from_slice(&v.to_le_bytes());
    }
    let mut body = Vec::new();
    let mut off = 6 + 16 * sizes.len();
    for &s in sizes {
        let img = [s as u8, (s >> 8) as u8];
        let w = if s >= 256 { 0 } else { s as u8 };
        head.extend_from_slice(&[w, w, 0, 0]);
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&32u16.to_le_bytes());
        head.extend_from_slice(&(img.len() as u32).to_le_bytes());
        head.extend_from_slice(&(off as u32).to_le_bytes());
        body.extend_from_slice(&img);
        off += img.len();
    }
    head.extend(body);
    head
}

#[test]
fn icons() {
    let ico = make_ico(&[16, 256, 32, 48]);
    for (size, want) in [(16, 16), (20, 32), (32, 32), (40, 48), (64, 256), (300, 256)] {
        let img = icon_image(&ico, size).unwrap();
        assert_eq!(img[0] as usize | (img[1] as usize) << 8, want, "size {size}");
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend([1, 2, 3]);
    assert_eq!(icon_image(&png, 32), Some(&png[..]));
    let mut broken = make_ico(&[32]);
    broken[18..22].copy_from_slice(&1000u32.to_le_bytes());
    for bad in [vec![], b"GIF89a".to_vec(), make_ico(&[]), broken] {
        assert!(!check_icon(&bad), "{bad:?} accepted");
    }
}

#[test]
fn run_forwards_to_caller() {
    let log = SharedBuf::default();
    let m = Mock::from_json(r#"{"confirm": [true], "progress": [{"cancelAfter": 2}]}"#, Some(log.sink())).unwrap();
    struct Rec {
        m: Mock,
        calls: std::sync::Mutex<Vec<(String, std::thread::ThreadId)>>,
    }
    impl Rec {
        fn note(&self, n: &str) {
            self.calls.lock().unwrap().push((n.into(), std::thread::current().id()));
        }
    }
    impl Ui for Rec {
        fn name(&self) -> String {
            self.m.name()
        }
        fn alert(&self, a: &str, b: &str) {
            self.m.alert(a, b)
        }
        fn confirm(&self, a: &str, b: &str) -> bool {
            self.note("confirm");
            self.m.confirm(a, b)
        }
        fn prompt(&self, a: &str, b: &str, c: &str) -> Option<String> {
            self.m.prompt(a, b, c)
        }
        fn open_files(&self, o: &PickerOptions) -> Vec<String> {
            self.m.open_files(o)
        }
        fn save_file(&self, o: &PickerOptions) -> Option<String> {
            self.m.save_file(o)
        }
        fn pick_directory(&self, o: &PickerOptions) -> Option<String> {
            self.m.pick_directory(o)
        }
        fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress> {
            self.note("progress");
            self.m.progress(o)
        }
        fn show_error(&self, a: &str, b: &str) {
            self.m.show_error(a, b)
        }
        fn show_in_folder(&self, p: &[String]) -> crate::Result<()> {
            self.m.show_in_folder(p)
        }
        fn open_path(&self, t: &str) -> crate::Result<()> {
            self.m.open_path(t)
        }
        fn open_window(&self, o: WindowOptions) -> crate::Result<Arc<dyn Window>> {
            self.note("window");
            self.m.open_window(o)
        }
        fn supports_windows(&self) -> bool {
            false
        }
        fn form(&self, o: &FormOptions) -> Option<serde_json::Map<String, Value>> {
            self.m.form(o)
        }
        fn table(&self, o: &TableOptions) -> Option<Vec<i64>> {
            self.m.table(o)
        }
    }
    let rec = Arc::new(Rec { m, calls: Default::default() });
    let caller = std::thread::current().id();
    let closed = Arc::new(AtomicBool::new(false));
    let c2 = closed.clone();
    let script = run(rec.clone(), move |su| {
        assert!(su.name() == "mock" && su.confirm("t", "m"), "confirm was not forwarded");
        let p = su.progress(&ProgressOptions { title: "p".into(), total: 3, ..Default::default() });
        p.update(1, 3, "");
        assert!(!p.cancelled(), "cancelled too early");
        p.update(2, 3, "");
        assert!(p.cancelled(), "updates must be applied before cancelled is answered");
        p.close();
        let w = su
            .open_window(WindowOptions { title: "w".into(), on_closed: Some(Arc::new(move || c2.store(true, Ordering::SeqCst))), ..Default::default() })
            .unwrap();
        w.post("hello".into()); // after close: ignored
        w.close();
        std::thread::current().id()
    });
    assert!(closed.load(Ordering::SeqCst));
    assert_ne!(script, caller, "the script ran on the calling thread");
    let calls = rec.calls.lock().unwrap();
    assert_eq!(calls.len(), 3, "{calls:?}");
    for (name, id) in calls.iter() {
        assert_eq!(*id, caller, "{name} did not run on the caller");
    }
    assert!(log.text().contains("progress closed at 2/3"), "{}", log.text());
}

/// The bundled Font Awesome stylesheet finds every font it refers to.
#[test]
fn font_awesome() {
    const CSS: &str = "fontawesome/css/all.min.css";
    let src = String::from_utf8_lossy(builtin_file(CSS).expect(CSS)).into_owned();
    let re = regex::Regex::new(r"url\(([^)]+)\)").unwrap();
    let urls: Vec<&str> = re.captures_iter(&src).map(|c| c.get(1).unwrap().as_str()).collect();
    assert!(!urls.is_empty(), "no url() in {CSS}");
    for u in urls {
        let mut parts: Vec<&str> = vec!["fontawesome", "css"];
        for seg in u.trim_matches(['"', '\'']).split('/') {
            match seg {
                ".." => {
                    parts.pop();
                }
                "." | "" => {}
                s => parts.push(s),
            }
        }
        let name = parts.join("/");
        assert!(builtin_file(&name).is_some(), "{CSS} refers to {u} ({name}): not found");
    }
}

#[test]
fn mock_and_console_notify() {
    let _l = lock_lang("ja");
    let log = SharedBuf::default();
    Mock::from_json("", Some(log.sink())).unwrap().notify("集計", "完了しました");
    assert_eq!(log.text(), "[mock] notify(\"集計\", \"完了しました\")\n");
    let out = SharedBuf::default();
    console("", &out).notify("集計", "完了しました");
    assert_eq!(out.text(), "[集計] 完了しました\n");
}
