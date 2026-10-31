//! Day.js (https://day.js.org, MIT) with the locales in dayjs/locale (one
//! per language in i18n/locales besides English, which is built in) and the
//! plugins in dayjs/plugin, unmodified from the npm package (version in
//! dayjs/README.md). To update, copy dayjs.min.js and the same locale and
//! plugin files from a newer package.

/// Replaced in prelude.js by bundle().
pub const PLACEHOLDER: &str = "/*@dayjs@*/null";

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// A JavaScript expression that evaluates to { dayjs, locales, plugins }
/// without defining any globals: each UMD file runs as a CommonJS module
/// (module, exports and require are provided).
pub fn bundle() -> String {
    let files = crate::ui::dayjs_files();
    let names = |dir: &str| -> Vec<String> {
        let mut v: Vec<String> = files
            .iter()
            .filter_map(|(n, _)| n.strip_prefix(&format!("{dir}/")).and_then(|r| r.strip_suffix(".js")).map(str::to_string))
            .collect();
        v.sort();
        v
    };
    let file = |name: &str| -> String {
        let d = files.iter().find(|(n, _)| *n == name).map(|(_, d)| *d).unwrap_or_else(|| panic!("dayjs: {name} is missing"));
        String::from_utf8_lossy(d).into_owned()
    };
    let mut b = String::new();
    b.push_str("(function () { const mods = {}; const require = (name) => mods[name]; ");
    b.push_str("const load = (name, fn) => { const module = { exports: {} }; fn(module, module.exports, require); mods[name] = module.exports; }; ");
    let mut module = |name: &str, path: &str| {
        b.push_str(&format!("load({}, function (module, exports, require) {{\n", quote(name)));
        b.push_str(&file(path));
        b.push_str("\n});\n");
    };
    module("dayjs", "dayjs.min.js");
    let locales = names("locale");
    for l in &locales {
        module(&format!("dayjs/locale/{l}"), &format!("locale/{l}.js"));
    }
    let plugins = names("plugin");
    for p in &plugins {
        module(&format!("dayjs/plugin/{p}"), &format!("plugin/{p}.js"));
    }
    b.push_str("return { dayjs: mods.dayjs, locales: [");
    b.push_str(&locales.iter().map(|l| quote(l)).collect::<Vec<_>>().join(", "));
    b.push_str("], plugins: [");
    b.push_str(&plugins.iter().map(|p| format!("mods[{}]", quote(&format!("dayjs/plugin/{p}")))).collect::<Vec<_>>().join(", "));
    b.push_str("] }; })()");
    b
}
