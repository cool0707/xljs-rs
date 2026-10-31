use super::*;
use crate::testutil::lock_lang;

#[test]
fn english_unchanged() {
    let _l = lock_lang("en");
    let s = "TypeError: Cannot read property 'foo' of undefined";
    assert_eq!(text(s), s);
    assert_eq!(t("ui.copy", &[]), "Copy");
}

#[test]
fn language_selection() {
    let _l = lock_lang("ja");
    for (tag, want) in [("ja", "ja"), ("ja-JP", "ja"), ("ja_JP.UTF-8", "ja"), ("EN-us", "en")] {
        assert!(set_language(tag) && lang() == want, "{tag}");
    }
    assert!(!set_language("xx"), "unknown language must report false");
    assert_eq!(t("no.such.key", &[]), "no.such.key");
    set_language("ja");
    assert_eq!(crate::t!("cli.created", "a.exe"), "作成しました: a.exe");
    assert_eq!(current().excel.font, "游ゴシック");
}

#[test]
fn path_error() {
    let _l = lock_lang("ja");
    let e = std::fs::File::open("/definitely/missing/file.xlsx").unwrap_err();
    let msg = error(&crate::Error::path("open", "/definitely/missing/file.xlsx", e));
    assert!(msg.contains("file.xlsx") && msg.contains("開けません") && msg.contains("見つかりません"), "{msg}");
}

/// Every locale against en.json: no unknown keys, the same placeholders.
#[test]
fn locale_files() {
    let en = &locales()["en"];
    let ph = regex::Regex::new(r"\{\d+\}").unwrap();
    let marks = |s: &str| {
        let mut v: Vec<String> = ph.find_iter(s).map(|m| m.as_str().to_string()).collect();
        v.sort();
        v
    };
    for (lang, loc) in locales() {
        for (k, v) in &loc.messages {
            let base = en.messages.get(k).unwrap_or_else(|| panic!("{lang}.json: key {k:?} is not in en.json"));
            assert_eq!(marks(v), marks(base), "{lang}.json: {k:?} has different placeholders than en.json");
        }
    }
}

/// Every message key referenced from Rust or JavaScript code is defined in
/// en.json.
#[test]
fn keys_used_in_source_exist() {
    let root = crate::testutil::repo_path("src");
    let re = regex::Regex::new(r#"\b(?:t!|err!|throw_t|T)\(\s*"([a-zA-Z]+\.[\w.]+)""#).unwrap();
    let mut found = 0;
    for e in walkdir::WalkDir::new(root) {
        let e = e.unwrap();
        let p = e.path().to_string_lossy().to_string();
        if !(p.ends_with(".rs") || p.ends_with(".js")) || p.contains("fontawesome") || p.ends_with("tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(e.path()).unwrap_or_default();
        for c in re.captures_iter(&src) {
            found += 1;
            assert!(has_key(&c[1]), "{p}: key {:?} is not defined in locales/en.json", &c[1]);
        }
    }
    assert!(found >= 20, "only {found} keys found; is the pattern still right?");
}

#[test]
fn text_japanese() {
    let _l = lock_lang("ja");
    for (input, want) in [
        ("TypeError: cannot read property 'foo' of undefined", "TypeError: undefined のプロパティ 'foo' は読み取れません"),
        ("ReferenceError: hoge is not defined", "ReferenceError: hoge は定義されていません"),
        ("TypeError: obj.run is not a function", "TypeError: obj.run は関数ではありません"),
        (r#"a.js:1:4: Could not resolve "./x.js""#, r#"a.js:1:4: "./x.js" が見つかりません（import のパスを確認してください）"#),
        ("a.js:2:1: Expected `)` but found `EOF`", "a.js:2:1: \")\" が必要ですが、ファイルが終わっています（括弧の閉じ忘れなど）"),
        ("sheet 売上 does not exist", r#"シート "売上" はありません"#),
        ("plain message", "plain message"),
    ] {
        assert_eq!(text(input), want, "{input}");
    }
}
