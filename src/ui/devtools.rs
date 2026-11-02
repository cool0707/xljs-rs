//! Settings of the developer tools for the debugger window (xljs --debug).
//! WebView2 has no API to choose the panel or the language the developer
//! tools open with, so they go into the profile's Preferences file before
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
    let Some(new) = merge(&old) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, new);
}

/// The Preferences JSON old with the developer tools opening on the Sources
/// panel and, unless chosen in the tools, in the browser's language (the
/// system's, see Language of the environment). None when old is not a JSON
/// object (it is left alone).
pub(crate) fn merge(old: &str) -> Option<String> {
    let mut v: Value = if old.trim().is_empty() { Value::Object(Map::new()) } else { serde_json::from_str(old).ok()? };
    let devtools = object(v.as_object_mut()?, "devtools")?;
    // The values are JSON texts. The selected panel is "panel-selected-tab"
    // in current versions, "panel-selectedTab" in older ones.
    let prefs = object(devtools, "preferences")?;
    for k in ["panel-selected-tab", "panel-selectedTab"] {
        prefs.insert(k.into(), Value::String("\"sources\"".into()));
    }
    let browser_language = Value::String("\"browserLanguage\"".into());
    prefs.entry("language").or_insert(browser_language.clone());
    // Synced settings live here while sync is off.
    object(devtools, "synced_preferences_sync_disabled")?.entry("language").or_insert(browser_language);
    serde_json::to_string(&v).ok()
}

fn object<'a>(m: &'a mut Map<String, Value>, key: &str) -> Option<&'a mut Map<String, Value>> {
    m.entry(key).or_insert_with(|| Value::Object(Map::new())).as_object_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_new_and_existing() {
        let v: Value = serde_json::from_str(&merge("").unwrap()).unwrap();
        assert_eq!(v["devtools"]["preferences"]["panel-selected-tab"], "\"sources\"");
        assert_eq!(v["devtools"]["preferences"]["language"], "\"browserLanguage\"");
        assert_eq!(v["devtools"]["synced_preferences_sync_disabled"]["language"], "\"browserLanguage\"");

        let old = r#"{"a":1,"devtools":{"preferences":{"panel-selected-tab":"\"console\"","language":"\"en-US\"","x":"1"}}}"#;
        let v: Value = serde_json::from_str(&merge(old).unwrap()).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["devtools"]["preferences"]["panel-selected-tab"], "\"sources\"");
        // A language chosen in the tools stays.
        assert_eq!(v["devtools"]["preferences"]["language"], "\"en-US\"");
        assert_eq!(v["devtools"]["preferences"]["x"], "1");

        assert!(merge("not json").is_none());
        assert!(merge("[]").is_none());
        assert!(merge(r#"{"devtools":1}"#).is_none());
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
