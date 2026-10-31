use super::*;
use crate::testutil::{lock_lang, repo_path};

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Want {
    name: String,
    kind: String,
    code: String,
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(repo_path(&format!("testdata/vba/{name}"))).unwrap()
}

fn xlsm() -> Vec<u8> {
    std::fs::read(repo_path("testdata/scripts/macro.xlsm")).unwrap()
}

fn check_project(p: Option<Project>, sheets: bool) {
    let p = p.expect("no project");
    assert_eq!((p.name.as_str(), p.code_page), ("VBAProject", 932));
    let want: Vec<Want> = serde_json::from_slice(&fixture("modules.json")).unwrap();
    assert_eq!(p.modules.len(), want.len());
    let types = HashMap::from([("Module", STANDARD), ("Document", DOCUMENT), ("Class", CLASS), ("BaseClass", FORM)]);
    for (m, w) in p.modules.iter().zip(&want) {
        assert_eq!((m.name.as_str(), m.kind.as_str()), (w.name.as_str(), types[w.kind.as_str()]));
        assert_eq!(m.code, w.code, "module {}", m.name);
    }
    if sheets {
        assert_eq!(p.modules[1].sheet, "売上");
        assert_eq!(p.modules[2].sheet, "設定");
    }
    assert_eq!(p.modules[0].sheet, "");
}

#[test]
fn read_xlsm() {
    check_project(read(&xlsm()).unwrap(), true);
}

#[test]
fn read_xls() {
    check_project(read(&fixture("macro.xls")).unwrap(), false);
}

#[test]
fn read_bin() {
    check_project(read(&fixture("vbaProject.bin")).unwrap(), false);
}

#[test]
fn no_macros() {
    let mut buf = std::io::Cursor::new(Vec::new());
    umya_spreadsheet::writer::xlsx::write_writer(&umya_spreadsheet::new_file(), &mut buf).unwrap();
    assert!(read(buf.get_ref()).unwrap().is_none());
    assert!(read(&fixture("novba.xls")).unwrap().is_none());
}

#[test]
fn errors() {
    assert_eq!(read(b"a,b\n1,2\n").unwrap_err().key(), Some("vba.unsupported"));
    assert_eq!(read(&fixture("encrypted.cfb")).unwrap_err().key(), Some("vba.encrypted"));
    assert!(read_file("does-not-exist.xlsm").is_err());
}

#[test]
fn decompress_container() {
    // The example of [MS-OVBA] 3.2.3.
    let input = [
        0x01, 0x2F, 0xB0, 0x00, 0x23, 0x61, 0x61, 0x61, 0x62, 0x63, 0x64, 0x65, 0x82, 0x66, 0x00, 0x70, 0x61, 0x67, 0x68, 0x69,
        0x6A, 0x01, 0x38, 0x08, 0x61, 0x6B, 0x6C, 0x00, 0x30, 0x6D, 0x6E, 0x6F, 0x70, 0x06, 0x71, 0x02, 0x70, 0x04, 0x10, 0x72,
        0x73, 0x74, 0x75, 0x76, 0x10, 0x77, 0x78, 0x79, 0x7A, 0x00, 0x3C,
    ];
    assert_eq!(decompress(&input).unwrap(), b"#aaabcdefaaaaghijaaaaaklaaamnopqaaaaaaaaaaaarstuvwxyzaaa");
    // An uncompressed chunk.
    let raw = b"0123456789abcdef".repeat(256);
    let mut chunk = vec![0x01, 0xFF, 0x3F];
    chunk.extend_from_slice(&raw);
    assert_eq!(decompress(&chunk).unwrap(), raw);
    // Several chunks (compressed by the Go implementation).
    let want: Vec<Want> = serde_json::from_slice(&fixture("modules.json")).unwrap();
    let long = &want.iter().find(|w| w.name == "集計").unwrap().code;
    assert_eq!(decompress(&fixture("compress.bin")).unwrap(), long.as_bytes());
    for bad in [&[][..], &[0x02], &[0x01, 0x03, 0xB0, 0x01, 0x00, 0x10]] {
        assert!(decompress(bad).is_err(), "{bad:x?}");
    }
}

#[test]
fn markdown_text() {
    let _l = lock_lang("ja");
    let p = read(&xlsm()).unwrap().unwrap();
    let text = text("売上.xlsm", &p);
    for want in [
        "# 売上.xlsm の VBA コード\n",
        "VBA プロジェクト「VBAProject」のコードです（モジュール 7 個）",
        "## ThisWorkbook（ブックのモジュール）\n\n```vba\nPrivate Sub Workbook_Open()\n    MsgBox \"ようこそ\"\nEnd Sub\n```\n",
        "## Sheet1（シート「売上」のモジュール）\n\n```vba\nOption Explicit\n\nPrivate Sub Worksheet_Change(",
        "## Module1（標準モジュール）",
        "## 集計（標準モジュール）",
        "## Class1（クラスモジュール）",
        "## UserForm1（ユーザーフォーム。",
        "コードのないモジュール: Sheet2\n",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    assert!(!text.contains("Attribute VB_"));
    // Backticks in the code get a longer fence.
    let p = Project {
        name: "P".into(),
        modules: vec![Module { name: "M".into(), kind: STANDARD.into(), code: "s = \"```\"\n".into(), ..Default::default() }],
        ..Default::default()
    };
    assert!(super::text("a.xlsm", &p).contains("````vba\ns = \"```\"\n````\n"));
}
