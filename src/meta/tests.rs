use super::*;
use crate::i18n;

#[test]
fn parse_block() {
    let src = "\u{FEFF}#!/usr/bin/env xljs\n// Sales tool\n\n// ==xljs==\n// @mode    app            // console（既定）| app\n// @html    ui\\index.html\n// @assets  ui/ img\n// @assets  data/list.csv\n// @title   売上集計ツール // comment\n// @size    960x640\n// @lang    ja\n// @icon    img/App.ICO\n// @titlebar Custom\n// @author  someone\n// free text\n// ==/xljs==\n// @mode console\nconsole.log(1);\n";
    let (m, warns) = parse("a.js", src.as_bytes()).unwrap();
    let want = Meta {
        mode: "app".into(),
        html: "ui/index.html".into(),
        assets: vec!["ui".into(), "img".into(), "data/list.csv".into()],
        title: "売上集計ツール".into(),
        width: 960,
        height: 640,
        lang: "ja".into(),
        icon: "img/App.ICO".into(),
        title_bar: "custom".into(),
    };
    assert_eq!(m, want);
    assert!(warns.len() == 1 && warns[0].contains("@author") && warns[0].contains("a.js:14"), "{warns:?}");
    assert!(m.is_app());
}

#[test]
fn parse_no_block() {
    for src in ["", "console.log(1)\n// ==xljs==\n// @mode app\n// ==/xljs==\n", "/* x */\n// ==xljs==\n// @mode app\n"] {
        let (m, warns) = parse("a.js", src.as_bytes()).unwrap();
        assert!(warns.is_empty() && !m.is_app(), "{src:?}");
    }
}

#[test]
fn parse_errors() {
    let _l = crate::test_lang("en");
    for (src, want) in [
        ("// ==xljs==\n// @mode gui\n// ==/xljs==\n", "a.js:2"),
        ("// ==xljs==\n// @size big\n// ==/xljs==\n", "@size"),
        ("// ==xljs==\n// @html ../x.html\n// ==/xljs==\n", "../x.html"),
        ("// ==xljs==\n// @assets C:/x\n// ==/xljs==\n", "C:/x"),
        ("// ==xljs==\n// @mode app\n", "==/xljs=="),
        ("// ==xljs==\n// @icon ui/app.bmp\n// ==/xljs==\n", "@icon"),
        ("// ==xljs==\n// @icon ../app.ico\n// ==/xljs==\n", "../app.ico"),
        ("// ==xljs==\n// @titlebar none\n// ==/xljs==\n", "@titlebar"),
    ] {
        let e = parse("a.js", src.as_bytes()).unwrap_err();
        assert!(e.to_string().contains(want), "{src:?}: {e}");
    }
    let _ = i18n::lang();
}

#[test]
fn asset_fs() {
    let dir = tempdir();
    for f in ["main.js", "ui/index.html", "ui/css/a.css", "uix/b.txt", "page.html"] {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, f).unwrap();
    }
    let m = Meta { html: "page.html".into(), assets: vec!["ui".into()], icon: "ui/app.ico".into(), ..Default::default() };
    assert_eq!(m.asset_roots(), vec!["ui".to_string(), "page.html".to_string()]);
    let fsys = m.asset_fs(&dir);
    for f in ["ui/index.html", "ui/css/a.css", "page.html"] {
        assert_eq!(fsys.read(f).unwrap(), f.as_bytes(), "{f}");
    }
    for f in ["main.js", "uix/b.txt", "../x", "/etc/passwd"] {
        assert!(fsys.read(f).is_err(), "{f} should not be visible");
    }
    assert_eq!(fsys.files("ui").unwrap(), vec!["ui/css/a.css".to_string(), "ui/index.html".to_string()]);
    assert_eq!(Meta { icon: "app.ico".into(), ..Default::default() }.asset_roots(), vec!["app.ico".to_string()]);
    let m = Meta { html: "ui/index.html".into(), assets: vec!["ui".into()], ..Default::default() };
    assert_eq!(m.asset_roots(), vec!["ui".to_string()]);
    std::fs::remove_dir_all(&dir).ok();
}

fn tempdir() -> std::path::PathBuf {
    crate::testutil::tempdir()
}
