//! Localized messages. Each language is one JSON file in `locales/`
//! (embedded at build time); `en.json` is the fallback for every key. See
//! `locales/README.md` for how to add a language.

mod translate;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use regex::Regex;
use serde::Deserialize;

pub use translate::{error, text};
pub(crate) use translate::os_text as translate_os_text;

/// Defaults for workbooks created by scripts.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ExcelDefaults {
    #[serde(default)]
    pub font: String,
}

/// One language file.
#[derive(Debug, Deserialize)]
pub struct Locale {
    pub lang: String,
    #[serde(default)]
    pub name: String,
    /// Language tags (or prefixes such as "ja") that select this locale,
    /// compared case-insensitively ("ja-JP", "ja_JP.UTF-8" → "ja").
    #[serde(default, rename = "match")]
    pub matches: Vec<String>,
    #[serde(default)]
    pub excel: ExcelDefaults,
    #[serde(default)]
    pub messages: HashMap<String, String>,
    /// `[regexp, replacement]` pairs that translate English messages
    /// produced by libraries (JavaScript engine, bundler, Excel library).
    #[serde(default)]
    pub translations: Vec<(String, String)>,
    #[serde(skip)]
    rules: Vec<Regex>,
}

const FALLBACK: &str = "en";

static LOCALE_FILES: &[(&str, &str)] = &[
    ("en.json", include_str!("locales/en.json")),
    ("ja.json", include_str!("locales/ja.json")),
];

static LOCALES: LazyLock<HashMap<String, Arc<Locale>>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for (name, data) in LOCALE_FILES {
        let loc = parse_locale(data).unwrap_or_else(|e| panic!("i18n: locales/{name}: {e}"));
        m.insert(loc.lang.clone(), Arc::new(loc));
    }
    assert!(m.contains_key(FALLBACK), "i18n: locales/en.json is missing");
    m
});

static CURRENT: LazyLock<RwLock<Arc<Locale>>> = LazyLock::new(|| {
    let loc = pick(std::env::var("XLJS_LANG").ok().as_deref().unwrap_or(""));
    RwLock::new(loc)
});

fn parse_locale(data: &str) -> Result<Locale, String> {
    let mut loc: Locale = serde_json::from_str(data).map_err(|e| e.to_string())?;
    if loc.lang.is_empty() {
        return Err(r#""lang" is empty"#.into());
    }
    for (re, _) in &loc.translations {
        let r = Regex::new(re).map_err(|e| format!("translation {re:?}: {e}"))?;
        loc.rules.push(r);
    }
    Ok(loc)
}

fn norm_tag(s: &str) -> String {
    let mut s = s.trim().to_lowercase();
    if let Some(i) = s.find(['.', '@']) {
        s.truncate(i); // ja_JP.UTF-8
    }
    s.replace('_', "-")
}

/// The locale for a language tag, trying the full tag first and then its
/// primary language ("ja-jp" → "ja").
fn find(tag: &str) -> Option<Arc<Locale>> {
    let tag = norm_tag(tag);
    if tag.is_empty() {
        return None;
    }
    let primary = tag.split('-').next().unwrap_or("").to_string();
    for t in [tag.as_str(), primary.as_str()] {
        if let Some(loc) = LOCALES.get(t) {
            return Some(loc.clone());
        }
        let mut langs: Vec<_> = LOCALES.values().collect();
        langs.sort_by(|a, b| a.lang.cmp(&b.lang));
        for loc in langs {
            if loc.matches.iter().any(|m| norm_tag(m) == t) {
                return Some(loc.clone());
            }
        }
    }
    None
}

fn is_auto(tag: &str) -> bool {
    tag.is_empty() || tag.eq_ignore_ascii_case("auto")
}

fn pick(tag: &str) -> Arc<Locale> {
    if !is_auto(tag)
        && let Some(loc) = find(tag)
    {
        return loc;
    }
    find(&system_language()).unwrap_or_else(|| LOCALES[FALLBACK].clone())
}

/// Selects a language by tag ("ja", "en-US", ...) or "auto"/"" for the
/// system language. It returns false (and keeps the system/fallback
/// language) when no locale matches.
pub fn set_language(tag: &str) -> bool {
    let found = !is_auto(tag) && find(tag).is_some();
    *CURRENT.write().unwrap() = pick(tag);
    found || is_auto(tag)
}

/// The active locale.
pub fn current() -> Arc<Locale> {
    CURRENT.read().unwrap().clone()
}

/// The active language code, e.g. "ja".
pub fn lang() -> String {
    current().lang.clone()
}

/// The codes of all embedded languages, sorted.
pub fn available() -> Vec<String> {
    let mut v: Vec<String> = LOCALES.keys().cloned().collect();
    v.sort();
    v
}

/// The raw message for key in the active language, falling back to
/// English and finally to the key itself.
pub fn message(key: &str) -> String {
    let cur = current();
    if let Some(m) = cur.messages.get(key) {
        return m.clone();
    }
    if let Some(m) = LOCALES[FALLBACK].messages.get(key) {
        return m.clone();
    }
    key.to_string()
}

/// Whether key is defined in English (all keys must be).
pub fn has_key(key: &str) -> bool {
    LOCALES[FALLBACK].messages.contains_key(key)
}

static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\d+)\}").unwrap());

/// Replaces {0}, {1}, ... in msg with args.
pub fn format(msg: &str, args: &[String]) -> String {
    if args.is_empty() {
        return msg.to_string();
    }
    PLACEHOLDER
        .replace_all(msg, |c: &regex::Captures| {
            let i: usize = c[1].parse().unwrap_or(usize::MAX);
            args.get(i).cloned().unwrap_or_else(|| c[0].to_string())
        })
        .into_owned()
}

/// The message for key with {0}, {1}, ... replaced by args.
pub fn t(key: &str, args: &[String]) -> String {
    format(&message(key), args)
}

/// `t!("key", a, b)`: the localized message with its arguments (anything
/// that implements Display).
#[macro_export]
macro_rules! t {
    ($key:expr) => { $crate::i18n::t($key, &[]) };
    ($key:expr, $($arg:expr),+ $(,)?) => { $crate::i18n::t($key, &[$(($arg).to_string()),+]) };
}

/// `err!("key", args...)`: an [`crate::Error`] with the localized message.
#[macro_export]
macro_rules! err {
    ($key:expr) => { $crate::Error::localized($key, $crate::i18n::t($key, &[])) };
    ($key:expr, $($arg:expr),+ $(,)?) => {
        $crate::Error::localized($key, $crate::i18n::t($key, &[$(($arg).to_string()),+]))
    };
}

/// All messages of the active language merged over the English ones.
pub fn messages() -> HashMap<String, String> {
    let mut out = LOCALES[FALLBACK].messages.clone();
    for (k, v) in &current().messages {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// The locales by language code (tests).
pub fn locales() -> &'static HashMap<String, Arc<Locale>> {
    &LOCALES
}

impl Locale {
    pub(crate) fn rules(&self) -> impl Iterator<Item = (&Regex, &str)> {
        self.rules.iter().zip(self.translations.iter().map(|(_, r)| r.as_str()))
    }
}

/// The language tag of the system: the Windows display language ("ja-JP"),
/// elsewhere the POSIX locale variables.
#[cfg(windows)]
fn system_language() -> String {
    use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    unsafe {
        let lid = GetUserDefaultUILanguage();
        let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
        let n = LCIDToLocaleName(lid as u32, Some(&mut buf), 0);
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..(n as usize - 1)])
    }
}

#[cfg(not(windows))]
fn system_language() -> String {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(k)
            && !v.is_empty()
            && v != "C"
            && v != "POSIX"
        {
            return v;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests;
