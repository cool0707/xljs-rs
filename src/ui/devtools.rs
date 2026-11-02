//! Settings of the developer tools for the debugger window (xljs --debug).
//! WebView2 has no API to choose the panel or the language the developer
//! tools open with, so they (and the browser's languages, which the profile
//! keeps from its creation) go into the profile's Preferences file before
//! the browser starts: the debugger uses a profile of its own (see
//! native::webview::data_folder), which no running browser holds.

use serde_json::{Map, Value};

/// The Preferences file of a WebView2 user data folder.
pub(crate) fn preferences_path(user_data: &str) -> std::path::PathBuf {
    std::path::Path::new(user_data).join("EBWebView").join("Default").join("Preferences")
}

/// Writes the settings into the Preferences file of user_data; errors are
/// ignored (the tools then open as before).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn prepare(user_data: &str) {
    let path = preferences_path(user_data);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let lang = crate::i18n::system_language();
    let Some(new) = merge(&old, &locale(&lang), &accept_languages(&lang)) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, new);
}

/// The locales of the developer tools (Chromium's DevTools front end).
const LOCALES: &[&str] = &[
    "af", "am", "ar", "as", "az", "be", "bg", "bn", "bs", "ca", "cs", "cy", "da", "de", "el", "en-GB", "en-US", "es", "es-419", "et", "eu", "fa", "fi", "fil", "fr",
    "fr-CA", "gl", "gu", "he", "hi", "hr", "hu", "hy", "id", "is", "it", "ja", "ka", "kk", "km", "kn", "ko", "ky", "lo", "lt", "lv", "mk", "ml", "mn", "mr", "ms",
    "my", "ne", "nl", "no", "or", "pa", "pl", "pt", "pt-PT", "ro", "ru", "si", "sk", "sl", "sq", "sr", "sr-Latn", "sv", "sw", "ta", "te", "th", "tr", "uk", "ur",
    "uz", "vi", "zh", "zh-HK", "zh-TW", "zu",
];

/// The developer tools' locale for a system language such as "ja-JP"
/// ("ja"); empty when they have none.
pub(crate) fn locale(lang: &str) -> String {
    let lang = lang.split(['.', '@']).next().unwrap_or("").replace('_', "-");
    let lower = lang.to_ascii_lowercase();
    let special = match lower.as_str() {
        "zh-cn" | "zh-sg" | "zh-hans" | "zh-hans-cn" => Some("zh"),
        "zh-tw" | "zh-hant" | "zh-hant-tw" => Some("zh-TW"),
        "zh-hk" | "zh-mo" | "zh-hant-hk" => Some("zh-HK"),
        "nb" | "nb-no" | "nn" | "nn-no" => Some("no"),
        "pt-br" => Some("pt"),
        _ => None,
    };
    if let Some(l) = special {
        return l.into();
    }
    if let Some(l) = LOCALES.iter().find(|l| l.eq_ignore_ascii_case(&lower)) {
        return l.to_string();
    }
    let base = lower.split('-').next().unwrap_or("");
    match base {
        "en" => "en-US".into(),
        _ => LOCALES.iter().find(|l| **l == base).map(|l| l.to_string()).unwrap_or_default(),
    }
}

/// The browser's languages for a system language: "ja-JP" gives
/// "ja-JP,ja" (the language and its base); empty for none.
pub(crate) fn accept_languages(lang: &str) -> String {
    let lang = lang.split(['.', '@']).next().unwrap_or("").replace('_', "-");
    if lang.is_empty() || lang.eq_ignore_ascii_case("C") || lang.eq_ignore_ascii_case("POSIX") {
        return String::new();
    }
    match lang.split_once('-') {
        Some((base, _)) if !base.is_empty() => format!("{lang},{base}"),
        _ => lang,
    }
}

/// The Preferences JSON old with the developer tools opening on the Sources
/// panel and, unless another language was chosen in the tools, in locale.
/// WebView2 does not pass its Language to the tools, so "browserLanguage"
/// would be English; it is replaced too. An empty locale leaves the language
/// alone. The browser's languages (intl, which the profile keeps from its
/// creation and the tools' "Browser UI language" follows) become accept
/// unless it is empty. None when old is not a JSON object (it is left alone).
pub(crate) fn merge(old: &str, locale: &str, accept: &str) -> Option<String> {
    let mut v: Value = if old.trim().is_empty() { Value::Object(Map::new()) } else { serde_json::from_str(old).ok()? };
    if !accept.is_empty() {
        let intl = object(v.as_object_mut()?, "intl")?;
        for k in ["accept_languages", "selected_languages"] {
            intl.insert(k.into(), Value::String(accept.into()));
        }
    }
    let devtools = object(v.as_object_mut()?, "devtools")?;
    // The values are JSON texts. The selected panel is "panel-selected-tab"
    // in current versions, "panel-selectedTab" in older ones.
    let prefs = object(devtools, "preferences")?;
    for k in ["panel-selected-tab", "panel-selectedTab"] {
        prefs.insert(k.into(), Value::String("\"sources\"".into()));
    }
    if !locale.is_empty() {
        set_language(prefs, locale);
        // Synced settings live here while sync is off.
        set_language(object(devtools, "synced_preferences_sync_disabled")?, locale);
    }
    serde_json::to_string(&v).ok()
}

/// Sets the language unless one other than the browser's was chosen.
fn set_language(prefs: &mut Map<String, Value>, locale: &str) {
    let unset = match prefs.get("language") {
        None => true,
        Some(Value::String(s)) => s == "\"browserLanguage\"" || s.is_empty(),
        Some(_) => false,
    };
    if unset {
        prefs.insert("language".into(), Value::String(format!("\"{locale}\"")));
    }
}

fn object<'a>(m: &'a mut Map<String, Value>, key: &str) -> Option<&'a mut Map<String, Value>> {
    m.entry(key).or_insert_with(|| Value::Object(Map::new())).as_object_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_new_and_existing() {
        let v: Value = serde_json::from_str(&merge("", "ja", "ja-JP,ja").unwrap()).unwrap();
        assert_eq!(v["devtools"]["preferences"]["panel-selected-tab"], "\"sources\"");
        assert_eq!(v["devtools"]["preferences"]["language"], "\"ja\"");
        assert_eq!(v["devtools"]["synced_preferences_sync_disabled"]["language"], "\"ja\"");
        assert_eq!(v["intl"]["accept_languages"], "ja-JP,ja");
        assert_eq!(v["intl"]["selected_languages"], "ja-JP,ja");

        // The browser's language (written by earlier versions) is replaced.
        let old = r#"{"devtools":{"preferences":{"language":"\"browserLanguage\""}}}"#;
        let v: Value = serde_json::from_str(&merge(old, "ja", "").unwrap()).unwrap();
        assert_eq!(v["devtools"]["preferences"]["language"], "\"ja\"");

        // No locale: the language is left alone.
        let v: Value = serde_json::from_str(&merge("", "", "").unwrap()).unwrap();
        assert!(v["devtools"]["preferences"].get("language").is_none());

        let old = r#"{"a":1,"devtools":{"preferences":{"panel-selected-tab":"\"console\"","language":"\"en-US\"","x":"1"}}}"#;
        let v: Value = serde_json::from_str(&merge(old, "ja", "").unwrap()).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["devtools"]["preferences"]["panel-selected-tab"], "\"sources\"");
        // A language chosen in the tools stays.
        assert_eq!(v["devtools"]["preferences"]["language"], "\"en-US\"");
        assert_eq!(v["devtools"]["preferences"]["x"], "1");

        assert!(merge("not json", "ja", "").is_none());
        assert!(merge("[]", "ja", "").is_none());
        assert!(merge(r#"{"devtools":1}"#, "ja", "").is_none());
    }

    #[test]
    fn locales() {
        assert_eq!(locale("ja-JP"), "ja");
        assert_eq!(locale("ja_JP.UTF-8"), "ja");
        assert_eq!(locale("en-US"), "en-US");
        assert_eq!(locale("en-AU"), "en-US");
        assert_eq!(locale("en-GB"), "en-GB");
        assert_eq!(locale("zh-CN"), "zh");
        assert_eq!(locale("zh-TW"), "zh-TW");
        assert_eq!(locale("pt-BR"), "pt");
        assert_eq!(locale("de-DE"), "de");
        assert_eq!(locale("xx-YY"), "");
        assert_eq!(locale(""), "");
        assert_eq!(accept_languages("ja-JP"), "ja-JP,ja");
        assert_eq!(accept_languages("ja_JP.UTF-8"), "ja-JP,ja");
        assert_eq!(accept_languages("ja"), "ja");
        assert_eq!(accept_languages(""), "");
        assert_eq!(accept_languages("C"), "");
    }

    #[test]
    fn prepare_writes_file() {
        let dir = std::env::temp_dir().join(format!("xljs-devtools-{}", std::process::id()));
        let d = dir.to_string_lossy().into_owned();
        prepare(&d);
        let s = std::fs::read_to_string(preferences_path(&d)).unwrap();
        assert!(s.contains("panel-selected-tab"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
