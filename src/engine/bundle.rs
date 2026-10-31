//! The bundler: resolves the imports of the entry file and puts every
//! module into one script that the engine (and the debugger page) can run.
//! TypeScript is stripped and ES modules become async functions with
//! oxc's module runner transform (imports are awaited calls, exports are
//! getters on an object), so top-level await, live bindings and cyclic
//! imports keep working. npm packages in node_modules (ES modules and
//! CommonJS), JSON and .txt files can be imported as well.
//!
//! The result is an expression: an async function that takes a function
//! receiving the entry's exports (the functions windows can invoke) and
//! runs the entry. An inline source map maps it back to the original files.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use oxc::allocator::Allocator;
use oxc::codegen::{Codegen, CodegenOptions, CommentOptions};
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};
use oxc::transformer_plugins::ModuleRunnerTransform;
use oxc_sourcemap::{ConcatSourceMapBuilder, SourceMap};

use super::{BundleError, BundleMessage};
use crate::{Error, Result, err};

/// The parameter of the bundle's function that receives the entry's exports.
pub const EXPORTS_PARAM: &str = "__xljs_exports";

const EXTENSIONS: &[&str] = &[".ts", ".mts", ".js", ".mjs", ".cjs", ".json"];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Esm,
    Cjs,
    Json,
    Text,
}

struct Module {
    id: String,
    kind: Kind,
    code: String,
    map: Option<SourceMap<'static>>,
    deps: BTreeMap<String, String>, // specifier -> id
}

struct Bundler {
    root: PathBuf,
    with_sources: bool,
    modules: Vec<Module>,
    ids: HashMap<PathBuf, String>,
    errors: Vec<BundleMessage>,
}

/// Bundles the script at entry (see the module documentation).
pub fn bundle(entry: &str) -> Result<Vec<u8>> {
    bundle_with(entry, false)
}

/// bundle; with_sources puts the original files into the source map (for
/// the debugger, which has no other way to show them).
pub fn bundle_with(entry: &str, with_sources: bool) -> Result<Vec<u8>> {
    let abs = std::path::absolute(entry).map_err(|e| Error::path("open", entry, e))?;
    std::fs::metadata(&abs).map_err(|e| Error::path("open", entry, e))?;
    let abs = normalize(&abs);
    let root = abs.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut b = Bundler { root, with_sources, modules: Vec::new(), ids: HashMap::new(), errors: Vec::new() };
    let entry_id = b.add(&abs, true);
    if !b.errors.is_empty() {
        return Err(Error::Bundle(BundleError { messages: b.errors }));
    }
    Ok(b.emit(&entry_id).into_bytes())
}

/// Removes "." and ".." from an absolute path without touching the disk.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn line_col(src: &str, offset: usize) -> (usize, usize, String) {
    let offset = offset.min(src.len());
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = src[start..offset].chars().count();
    let end = src[offset..].find('\n').map(|i| offset + i).unwrap_or(src.len());
    (line, col, src[start..end].trim_end_matches('\r').to_string())
}

impl Bundler {
    /// The name of a file in messages and the source map: relative to the
    /// entry's folder, with slashes.
    fn display(&self, p: &Path) -> String {
        let rel = pathdiff(p, &self.root);
        rel.to_string_lossy().replace('\\', "/")
    }

    fn error(&mut self, file: &str, src: &str, offset: Option<usize>, text: String) {
        let loc = offset.map(|o| {
            let (line, col, line_text) = line_col(src, o);
            (file.to_string(), line, col, line_text)
        });
        self.errors.push(BundleMessage { location: loc, text });
    }

    /// Adds the module at path (once) and returns its id.
    fn add(&mut self, path: &Path, entry: bool) -> String {
        if let Some(id) = self.ids.get(path) {
            return id.clone();
        }
        let id = self.display(path);
        self.ids.insert(path.to_path_buf(), id.clone());
        let src = match std::fs::read(path) {
            Ok(b) => String::from_utf8_lossy(b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&b)).into_owned(),
            Err(e) => {
                self.errors.push(BundleMessage { location: None, text: crate::i18n::error(&Error::path("open", id.clone(), e)) });
                return id;
            }
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let idx = self.modules.len();
        self.modules.push(Module { id: id.clone(), kind: Kind::Esm, code: String::new(), map: None, deps: BTreeMap::new() });
        match ext.as_str() {
            "json" => match serde_json::from_str::<serde_json::Value>(&src) {
                Ok(v) => {
                    self.modules[idx].kind = Kind::Json;
                    self.modules[idx].code = v.to_string();
                }
                Err(e) => {
                    let off = line_offset(&src, e.line(), e.column());
                    self.error(&id, &src, Some(off), e.to_string());
                }
            },
            "txt" => {
                self.modules[idx].kind = Kind::Text;
                self.modules[idx].code = serde_json::to_string(&src).unwrap();
            }
            _ => self.add_script(idx, path, &id, &src, &ext, entry),
        }
        id
    }

    fn add_script(&mut self, idx: usize, path: &Path, id: &str, src: &str, ext: &str, entry: bool) {
        let alloc = Allocator::default();
        let ts = matches!(ext, "ts" | "mts" | "cts");
        let mut source_type = if ts { SourceType::ts() } else { SourceType::mjs() };
        if ext == "cjs" || ext == "cts" {
            source_type = source_type.with_commonjs(true);
        }
        let mut ret = Parser::new(&alloc, src, source_type).parse();
        if !ret.diagnostics.is_empty() && !entry && ext == "js" {
            // Old CommonJS packages may not parse as modules.
            let r2 = Parser::new(&alloc, src, SourceType::cjs()).parse();
            if r2.diagnostics.is_empty() {
                ret = r2;
                source_type = SourceType::cjs();
            }
        }
        if !ret.diagnostics.is_empty() {
            for d in ret.diagnostics.iter() {
                let off = d.labels.first().map(|l| l.offset() as usize);
                let mut text = d.message.to_string();
                if let Some(help) = &d.help {
                    text = format!("{text} ({help})");
                }
                self.error(id, src, off, text);
            }
            return;
        }
        let cjs = !entry
            && (source_type.is_commonjs()
                || (!ret.module_record.has_module_syntax && looks_commonjs(src) && !ts));
        // Where each specifier first appears (for messages).
        let mut spans: HashMap<String, usize> = HashMap::new();
        for (spec, reqs) in ret.module_record.requested_modules.iter() {
            if let Some(r) = reqs.first() {
                spans.insert(spec.to_string(), r.span.start as usize);
            }
        }
        let mut program = ret.program;
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut scoping = scoping;
        if ts {
            let opts = TransformOptions::from_target("esnext").unwrap_or_default();
            let r = Transformer::new(&alloc, path, &opts).build_with_scoping(scoping, &mut program);
            if !r.diagnostics.is_empty() {
                for d in r.diagnostics.iter() {
                    let off = d.labels.first().map(|l| l.offset() as usize);
                    self.error(id, src, off, d.message.to_string());
                }
                return;
            }
            scoping = r.scoping;
        }
        let mut deps: Vec<String> = Vec::new();
        if cjs {
            deps = require_calls(src);
        } else {
            let (static_deps, dynamic_deps) = ModuleRunnerTransform::new().transform(&alloc, &mut program, scoping);
            for d in static_deps.iter().chain(dynamic_deps.iter()) {
                if let Some(s) = d.as_str() {
                    deps.push(s.to_string());
                }
            }
            deps.sort();
            deps.dedup();
        }
        let out = Codegen::new()
            .with_options(CodegenOptions {
                comments: CommentOptions::disabled(),
                source_map_path: Some(PathBuf::from(id)),
                ..CodegenOptions::default()
            })
            .with_source_text(src)
            .build(&program);
        let mut map = out.map.map(SourceMap::into_owned);
        if let Some(m) = map.as_mut() {
            m.set_sources([id]);
            if self.with_sources {
                m.set_source_contents(vec![Some(src)]);
                *m = m.clone().into_owned();
            }
        }
        self.modules[idx].kind = if cjs { Kind::Cjs } else { Kind::Esm };
        self.modules[idx].code = out.code;
        self.modules[idx].map = map;
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        for spec in deps {
            match self.resolve(&dir, &spec) {
                Some(p) => {
                    let dep = self.add(&p, false);
                    self.modules[idx].deps.insert(spec, dep);
                }
                None if cjs => {} // a require() that is never reached, e.g. of a Node.js module
                None => {
                    let off = spans.get(&spec).copied().or_else(|| src.find(&format!("\"{spec}\"")).or_else(|| src.find(&format!("'{spec}'"))));
                    self.error(id, src, off, format!("Could not resolve \"{spec}\""));
                }
            }
        }
    }

    /// Resolves an import specifier like esbuild: relative paths with the
    /// known extensions (and ".js" naming a ".ts" file), folders with an
    /// index file, packages in node_modules.
    fn resolve(&self, dir: &Path, spec: &str) -> Option<PathBuf> {
        if spec.starts_with("node:") {
            return None;
        }
        if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') || spec == "." || spec == ".." {
            return resolve_path(&normalize(&dir.join(spec)));
        }
        if Path::new(spec).is_absolute() {
            return resolve_path(&normalize(Path::new(spec)));
        }
        // A package: "name" or "@scope/name", then an optional subpath.
        let mut parts = spec.splitn(if spec.starts_with('@') { 3 } else { 2 }, '/');
        let name = if spec.starts_with('@') {
            format!("{}/{}", parts.next()?, parts.next()?)
        } else {
            parts.next()?.to_string()
        };
        let sub = parts.next().map(str::to_string);
        let mut d = Some(dir.to_path_buf());
        while let Some(cur) = d {
            let pkg = cur.join("node_modules").join(&name);
            if pkg.is_dir() {
                return resolve_package(&pkg, sub.as_deref());
            }
            d = cur.parent().map(Path::to_path_buf);
        }
        None
    }

    fn emit(&self, entry: &str) -> String {
        let mut code = String::new();
        let mut maps: Vec<(&SourceMap, u32)> = Vec::new();
        let mut line = 0u32;
        let mut push = |code: &mut String, s: &str, line: &mut u32| {
            code.push_str(s);
            *line += s.matches('\n').count() as u32;
        };
        push(&mut code, &format!("(async ({EXPORTS_PARAM}) => {{ const __xljs_m = Object.create(null);\n"), &mut line);
        for m in &self.modules {
            let deps = serde_json::to_string(&m.deps).unwrap();
            let key = serde_json::to_string(&m.id).unwrap();
            match m.kind {
                Kind::Esm => {
                    push(
                        &mut code,
                        &format!("__xljs_m[{key}] = [0, {deps}, async function (__vite_ssr_import__, __vite_ssr_dynamic_import__, __vite_ssr_exports__, __vite_ssr_exportAll__, __vite_ssr_import_meta__) {{ \"use strict\";\n"),
                        &mut line,
                    );
                    if let Some(map) = &m.map {
                        maps.push((map, line));
                    }
                    push(&mut code, &m.code, &mut line);
                    push(&mut code, "\n}];\n", &mut line);
                }
                Kind::Cjs => {
                    push(&mut code, &format!("__xljs_m[{key}] = [1, {deps}, function (module, exports, require) {{\n"), &mut line);
                    if let Some(map) = &m.map {
                        maps.push((map, line));
                    }
                    push(&mut code, &m.code, &mut line);
                    push(&mut code, "\n}];\n", &mut line);
                }
                Kind::Json | Kind::Text => {
                    push(&mut code, &format!("__xljs_m[{key}] = [2, {{}}, {}];\n", m.code), &mut line);
                }
            }
        }
        push(&mut code, RUNTIME, &mut line);
        push(&mut code, &format!("return __xljs_load({}, {EXPORTS_PARAM});\n}})", serde_json::to_string(entry).unwrap()), &mut line);
        let map = ConcatSourceMapBuilder::from_sourcemaps(&maps).into_sourcemap();
        code.push_str("\n//# sourceMappingURL=");
        code.push_str(&map.to_data_url());
        code.push('\n');
        code
    }
}

/// The module loader of the bundle (inside the bundle's function).
const RUNTIME: &str = r#"const __xljs_cache = Object.create(null);
const __xljs_toESM = (e) => {
  if (e !== null && typeof e === "object" && e[Symbol.toStringTag] === "Module") return e;
  const ns = Object.create(null);
  if (e !== null && (typeof e === "object" || typeof e === "function")) for (const k of Object.keys(e)) ns[k] = e[k];
  ns.default = e !== null && e !== undefined && e.__esModule ? e.default : e;
  return ns;
};
function __xljs_dep(id, spec) {
  const to = __xljs_m[id][1][spec];
  if (to === undefined) throw new Error(`Could not resolve "${spec}"`);
  return to;
}
function __xljs_require(id) {
  const c = __xljs_cache[id];
  if (c) return c.cjs ? c.module.exports : c.exports;
  const [kind, , fn] = __xljs_m[id];
  if (kind === 2) {
    __xljs_cache[id] = { exports: fn };
    return fn;
  }
  if (kind === 1) {
    const module = { exports: {} };
    __xljs_cache[id] = { cjs: true, module };
    fn.call(module.exports, module, module.exports, (spec) => __xljs_require(__xljs_dep(id, spec)));
    return module.exports;
  }
  throw new Error(`"${id}" is an ES module: use import instead of require`);
}
function __xljs_load(id, onExports) {
  const c = __xljs_cache[id];
  // A module that is still loading is in an import cycle: its exports
  // object fills up as it runs.
  if (c) return c.cjs ? __xljs_toESM(c.module.exports) : c.exports;
  const [kind, deps, fn] = __xljs_m[id];
  if (kind === 2) {
    const ns = Object.create(null);
    if (fn !== null && typeof fn === "object" && !Array.isArray(fn)) Object.assign(ns, fn);
    ns.default = fn;
    __xljs_cache[id] = { exports: ns };
    return ns;
  }
  if (kind === 1) return __xljs_toESM(__xljs_require(id));
  const exports = Object.create(null);
  Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
  __xljs_cache[id] = { exports };
  if (onExports) onExports(exports);
  const imp = (spec) => __xljs_load(__xljs_dep(id, spec));
  const dyn = async (spec) => {
    if (typeof spec !== "string" || deps[spec] === undefined) throw new Error(`Cannot import "${spec}" (only paths written as strings can be imported)`);
    return imp(spec);
  };
  const all = (src) => {
    for (const k in src) {
      if (k !== "default" && !(k in exports)) Object.defineProperty(exports, k, { enumerable: true, configurable: true, get() { return src[k]; } });
    }
  };
  return fn(imp, dyn, exports, all, { url: "file:///" + id });
}
"#;

fn line_offset(src: &str, line: usize, col: usize) -> usize {
    let mut off = 0;
    for (i, l) in src.split('\n').enumerate() {
        if i + 1 == line {
            return off + l.char_indices().nth(col.saturating_sub(1)).map(|(i, _)| i).unwrap_or(l.len());
        }
        off += l.len() + 1;
    }
    src.len()
}

/// Whether a script without import/export uses CommonJS.
fn looks_commonjs(src: &str) -> bool {
    src.contains("module.exports") || src.contains("exports.") || src.contains("require(")
}

/// The string arguments of require() calls.
fn require_calls(src: &str) -> Vec<String> {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"\brequire\s*\(\s*(?:"([^"]+)"|'([^']+)')\s*\)"#).unwrap());
    let mut out: Vec<String> = RE.captures_iter(src).filter_map(|c| c.get(1).or(c.get(2)).map(|m| m.as_str().to_string())).collect();
    out.sort();
    out.dedup();
    out
}

fn resolve_path(p: &Path) -> Option<PathBuf> {
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    let s = p.to_string_lossy();
    for ext in EXTENSIONS {
        let c = PathBuf::from(format!("{s}{ext}"));
        if c.is_file() {
            return Some(c);
        }
    }
    // "./a.js" may name "./a.ts" (TypeScript's convention).
    for (from, to) in [(".js", ".ts"), (".mjs", ".mts"), (".cjs", ".cts")] {
        if let Some(stem) = s.strip_suffix(from) {
            let c = PathBuf::from(format!("{stem}{to}"));
            if c.is_file() {
                return Some(c);
            }
        }
    }
    if p.is_dir() {
        if p.join("package.json").is_file() {
            return resolve_package(p, None);
        }
        for ext in EXTENSIONS {
            let c = p.join(format!("index{ext}"));
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// Resolves a package folder (and a subpath) with its package.json:
/// "exports" (conditions import, default), "module", "main".
fn resolve_package(pkg: &Path, sub: Option<&str>) -> Option<PathBuf> {
    let json: serde_json::Value =
        std::fs::read_to_string(pkg.join("package.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let key = match sub {
        Some(s) => format!("./{s}"),
        None => ".".to_string(),
    };
    if let Some(exports) = json.get("exports") {
        let target = match exports {
            serde_json::Value::Object(m) if m.keys().any(|k| k.starts_with('.')) => m.get(&key).cloned().or_else(|| {
                // "./*" patterns
                m.iter().find_map(|(k, v)| {
                    let (pre, post) = k.split_once('*')?;
                    let mid = key.strip_prefix(pre)?.strip_suffix(post)?;
                    Some(replace_star(v, mid))
                })
            }),
            other if key == "." => Some(other.clone()),
            _ => None,
        };
        if let Some(t) = target.and_then(|t| conditional(&t))
            && let Some(p) = resolve_path(&normalize(&pkg.join(t)))
        {
            return Some(p);
        }
    }
    if let Some(s) = sub {
        return resolve_path(&normalize(&pkg.join(s)));
    }
    for field in ["module", "main"] {
        if let Some(m) = json.get(field).and_then(|v| v.as_str())
            && let Some(p) = resolve_path(&normalize(&pkg.join(m)))
        {
            return Some(p);
        }
    }
    resolve_path(&pkg.join("index"))
}

fn replace_star(v: &serde_json::Value, mid: &str) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(s.replace('*', mid)),
        serde_json::Value::Object(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), replace_star(v, mid))).collect()),
        serde_json::Value::Array(a) => serde_json::Value::Array(a.iter().map(|v| replace_star(v, mid)).collect()),
        x => x.clone(),
    }
}

/// The target of a conditional export for the conditions import/default.
fn conditional(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => a.iter().find_map(conditional),
        serde_json::Value::Object(m) => {
            for (k, t) in m {
                if matches!(k.as_str(), "import" | "default" | "module") {
                    if let Some(s) = conditional(t) {
                        return Some(s);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// path relative to base (both absolute and normalized).
fn pathdiff(path: &Path, base: &Path) -> PathBuf {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return path.to_path_buf();
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c.as_os_str());
    }
    out
}

/// Bundles a script given as text (tests and `xljs check`).
#[allow(dead_code)]
pub(crate) fn check(entry: &str) -> Result<()> {
    bundle(entry).map(|_| ()).map_err(|e| match e {
        Error::Bundle(_) => e,
        other => err!("engine.internalError", other),
    })
}
