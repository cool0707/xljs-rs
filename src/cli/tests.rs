use std::sync::Arc;

use super::*;
use crate::testutil::{SharedBuf, lock_lang, repo_path, tempdir};

fn launch_with(answers: serde_json::Value) -> (i32, String) {
    let log = SharedBuf::default();
    let u = Arc::new(ui::Mock::from_json(&answers.to_string(), Some(log.sink())).unwrap());
    let code = launch::launch(&ConsoleState { has: true, own: false }, u, false);
    (code, log.text())
}

#[test]
fn launch_screen() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    let script = dir.join("hello.js");
    std::fs::write(&script, r#"ui.alert("こんにちは");"#).unwrap();
    let s = script.to_string_lossy().into_owned();

    let (code, log) = launch_with(serde_json::json!({}));
    assert!(code == 0 && log.contains("-> -1"), "cancel: {code}\n{log}");

    let (code, log) = launch_with(serde_json::json!({"choose": [0], "showOpenFilePicker": [[s]]}));
    assert!(code == 0 && log.contains("こんにちは"), "run: {code}\n{log}");

    let out = dir.join("hello.exe");
    let (code, log) = launch_with(serde_json::json!({"choose": [2], "showOpenFilePicker": [[s]], "showSaveFilePicker": [out.to_string_lossy()]}));
    assert_eq!(code, 0, "pack\n{log}");
    let p = pack::read(&out).unwrap().unwrap();
    assert_eq!(p.name, "hello.js");
    std::fs::remove_file(&out).ok();

    let book = dir.join("売上.xlsm");
    std::fs::copy(repo_path("testdata/scripts/macro.xlsm"), &book).unwrap();
    let (code, log) = launch_with(serde_json::json!({"choose": [3], "showOpenFilePicker": [[book.to_string_lossy()]]}));
    assert_eq!(code, 0, "vba\n{log}");
    assert!(dir.join("売上_VBA.txt").exists());
}

#[test]
fn drop_vba() {
    let _l = lock_lang("ja");
    let dir = tempdir();
    let (a, b) = (dir.join("売上.xlsm"), dir.join("b.xlsm"));
    std::fs::copy(repo_path("testdata/scripts/macro.xlsm"), &a).unwrap();
    std::fs::copy(repo_path("testdata/scripts/macro.xlsm"), &b).unwrap();
    let (a, b) = (a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned());
    assert!(vba::is_workbook(&a) && vba::is_workbook("x.XLS") && !vba::is_workbook("x.ts"));
    let u = Arc::new(ui::Mock::from_json("", None).unwrap());
    assert_eq!(vba::drop_vba(&ConsoleState { has: true, own: false }, u, &[a.clone(), b]), 0);
    let text = std::fs::read_to_string(dir.join("売上_VBA.txt")).unwrap();
    assert!(text.starts_with("# ") && text.matches("\n# ").count() == 1, "want two workbooks:\n{text}");
    assert!(text.contains("Sub 売上集計()"));

    let plain = dir.join("plain.xlsx");
    umya_spreadsheet::writer::xlsx::write(&umya_spreadsheet::new_file(), &plain).unwrap();
    let e = vba::vba_text(&[a, plain.to_string_lossy().into_owned()]).unwrap_err();
    assert_eq!(e.key(), Some("cli.vbaNone"));
    assert!(vba::vba_text(&[dir.join("missing.xlsm").to_string_lossy().into_owned()]).is_err());
}

#[test]
fn flags() {
    let args: Vec<String> = ["--ui=mock", "-lang", "en", "--debug", "s.js", "--debug", "x"].iter().map(|s| s.to_string()).collect();
    let (f, rest) = parse_flags(&args).unwrap();
    assert_eq!(f.ui.as_deref(), Some("mock"));
    assert_eq!(f.lang.as_deref(), Some("en"));
    assert_eq!(f.debug.as_deref(), Some("script"));
    assert_eq!(rest, vec!["s.js", "--debug", "x"]);
    assert!(parse_flags(&["--nope".to_string()]).is_err());
    let (out, rest) = split_out(&["a.js".into(), "-o".into(), "b.exe".into()]).unwrap();
    assert_eq!((out.as_deref(), rest), (Some("b.exe"), vec!["a.js".to_string()]));
}
