//! The development mode (xljs dev): a window with an AI chat (a web
//! browser pane: Copilot, Gemini, ChatGPT...) beside a canvas for the
//! script (web/dev.html: the code, the differences of an AI's answer, the
//! form or app preview, earlier versions and the output of runs).
//!
//! Files the chat downloads are matched to the script's files and shown as
//! differences; when the user accepts, the file is replaced and its old
//! content kept as a version (versions.rs). Runs are processes of their
//! own (runner.rs) with the endless loop check on and fs.remove asking
//! before deleting much; their errors come back with where they happened
//! and a request to paste into the chat (report.rs).
//!
//! The page and this side talk in JSON: the page sends {op, ...}, this
//! side answers with {type, ...} events.

pub mod project;
pub mod report;
pub mod runner;
pub mod versions;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::meta::AssetFs;
use crate::ui::{
    ASSET_ORIGIN, BUILTIN_PREFIX, BrowserCommand, BrowserEvent, BrowserOptions, BuiltinAssets, FileFilter, PickerOptions, Ui, Window, WindowOptions,
};
use crate::{Error, Result, err, i18n, t};
use project::{Project, read_text, write_text};
use runner::{Run, RunEvent};

/// The instructions for AI chats (docs/ai/prompt.md).
pub const AI_PROMPT: &str = include_str!("../../docs/ai/prompt.md");

/// The page of the development window.
pub const DEV_PAGE: &str = "dev.html";

/// The path under ASSET_ORIGIN that serves the script's folder to the app
/// preview.
const PROJECT_PREFIX: &str = "_project/";

/// The AI chats offered on the start of the chat pane: (name, URL).
pub const SERVICES: &[(&str, &str)] = &[
    ("Microsoft Copilot", "https://copilot.microsoft.com/"),
    ("Microsoft 365 Copilot", "https://m365.cloud.microsoft/chat"),
    ("Gemini", "https://gemini.google.com/app"),
    ("ChatGPT", "https://chatgpt.com/"),
    ("Claude", "https://claude.ai/new"),
];

/// How the development mode starts.
#[derive(Clone, Default)]
pub struct DevOptions {
    /// The script to open ("" = none; a folder = choose in it).
    pub path: String,
    /// The exe that runs scripts (this one).
    pub exe: PathBuf,
    /// Arguments for every run before the script (--lang=...).
    pub run_args: Vec<String>,
    /// Where the development mode keeps what is not a script's (the
    /// recent scripts).
    pub settings: PathBuf,
}

/// The settings file of the development mode in the user's profile.
pub fn default_settings() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("xljs").join("dev.json")
}

/// What reaches the session.
pub enum Event {
    Page(String, Vec<String>),
    Browser(BrowserEvent),
    Run(RunEvent),
    Closed,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct GlobalSettings {
    recent: Vec<String>,
}

/// The settings of a script's folder (.xljs/dev.json).
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ProjectSettings {
    /// The AI chat used for each script (entry name → URL).
    chat: HashMap<String, String>,
}

fn load<T: for<'a> Deserialize<'a> + Default>(p: &Path) -> T {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn store<T: Serialize>(p: &Path, v: &T) {
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let _ = write_text(p, &s);
    }
}

/// Serves the built-in pages and, for the app preview, the script's folder.
struct DevAssets {
    dir: Arc<Mutex<Option<PathBuf>>>,
}

impl AssetFs for DevAssets {
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        if let Some(rest) = name.strip_prefix(BUILTIN_PREFIX) {
            return BuiltinAssets.read(rest);
        }
        if let Some(rest) = name.strip_prefix(PROJECT_PREFIX)
            && let Some(dir) = self.dir.lock().unwrap().clone()
            && !rest.split('/').any(|p| p == ".." || p.starts_with('.'))
        {
            return std::fs::read(dir.join(rest));
        }
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string()))
    }
    fn files(&self, _root: &str) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// What a run is for.
#[derive(Clone, Copy, PartialEq, Debug)]
enum RunKind {
    Run,
    Debug,
    Preview,
}

struct Running {
    run: Run,
    kind: RunKind,
    stderr: String,
    preview: Option<PathBuf>,
}

/// Opens the development window and serves it until it is closed.
pub fn run(u: Arc<dyn Ui>, o: DevOptions) -> Result<i32> {
    if !u.supports_windows() {
        return Err(err!("dev.needsWindows"));
    }
    let (tx, rx) = mpsc::channel::<Event>();
    let downloads = std::env::temp_dir().join("xljs-dev").join(format!("{}", std::process::id()));
    let _ = std::fs::create_dir_all(&downloads);
    let dir = Arc::new(Mutex::new(None));
    let (t1, t2, t3) = (tx.clone(), tx.clone(), tx.clone());
    let w = u.open_window(WindowOptions {
        title: t!("dev.title"),
        width: 1280,
        height: 820,
        url: format!("{ASSET_ORIGIN}{BUILTIN_PREFIX}{DEV_PAGE}"),
        assets: Some(Arc::new(DevAssets { dir: dir.clone() })),
        debug: std::env::var("XLJS_DEV_DEBUG").is_ok_and(|v| !v.is_empty()),
        on_message: Some(Arc::new(move |m, files| {
            let _ = t1.send(Event::Page(m, files));
        })),
        on_closed: Some(Arc::new(move || {
            let _ = t2.send(Event::Closed);
        })),
        browser: Some(BrowserOptions {
            downloads: downloads.to_string_lossy().into_owned(),
            on_event: Arc::new(move |e| {
                let _ = t3.send(Event::Browser(e));
            }),
        }),
        ..Default::default()
    })?;
    let mut s = Session::new(u, w, o, tx, dir);
    while let Ok(ev) = rx.recv() {
        if !s.event(ev) {
            break;
        }
    }
    s.shutdown();
    let _ = std::fs::remove_dir_all(&downloads);
    Ok(0)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Request {
    op: String,
    path: String,
    file: String,
    text: String,
    note: String,
    url: String,
    app: bool,
    debug: bool,
    n: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

/// The state of the development window.
pub struct Session {
    u: Arc<dyn Ui>,
    w: Arc<dyn Window>,
    o: DevOptions,
    tx: Sender<Event>,
    dir: Arc<Mutex<Option<PathBuf>>>,
    project: Option<Project>,
    /// The file being edited (relative).
    current: String,
    /// The files saved in this session (the first save keeps a version).
    saved: Vec<String>,
    running: Option<Running>,
    next_run: u64,
    ready: bool,
    /// The page the chat pane shows ("" = none).
    chat_url: String,
}

impl Session {
    pub fn new(u: Arc<dyn Ui>, w: Arc<dyn Window>, o: DevOptions, tx: Sender<Event>, dir: Arc<Mutex<Option<PathBuf>>>) -> Session {
        Session { u, w, o, tx, dir, project: None, current: String::new(), saved: Vec::new(), running: None, next_run: 1, ready: false, chat_url: String::new() }
    }

    fn post(&self, v: Value) {
        self.w.post(v.to_string());
    }

    fn fail(&self, e: &Error) {
        self.post(json!({"type": "error", "message": i18n::error(e)}));
    }

    /// Handles an event; false when the window has closed.
    pub fn event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Closed => return false,
            Event::Page(m, files) => {
                let r: Request = match serde_json::from_str(&m) {
                    Ok(r) => r,
                    Err(_) => return true,
                };
                if let Err(e) = self.request(r, files) {
                    self.fail(&e);
                }
            }
            Event::Browser(b) => self.browser_event(b),
            Event::Run(r) => self.run_event(r),
        }
        true
    }

    fn project(&self) -> Result<&Project> {
        self.project.as_ref().ok_or_else(|| err!("dev.noScript"))
    }

    fn request(&mut self, r: Request, _files: Vec<String>) -> Result<()> {
        match r.op.as_str() {
            "ready" => {
                self.ready = true;
                self.post(self.init());
                let path = std::mem::take(&mut self.o.path);
                if !path.is_empty() {
                    if Path::new(&path).is_dir() {
                        self.open_dialog(&path);
                    } else {
                        self.open(&path)?;
                    }
                }
            }
            "open" => {
                if r.path.is_empty() {
                    let start = self.project.as_ref().map(|p| p.dir.to_string_lossy().into_owned()).unwrap_or_default();
                    self.open_dialog(&start);
                } else {
                    self.open(&r.path)?;
                }
            }
            "new" => self.create(r.app)?,
            "read" => {
                let p = self.project()?;
                let path = p.path(&r.file)?;
                let exists = path.is_file();
                let text = if exists { read_text(&path)? } else { String::new() };
                self.post(json!({"type": "file", "file": r.file, "text": text, "exists": exists}));
            }
            "edit" => {
                // The page shows this file now (for downloads and the chat).
                self.current = r.file;
            }
            "save" => {
                let p = self.project()?.clone();
                let path = p.path(&r.file)?;
                if !self.saved.contains(&r.file) {
                    versions::keep(&p.dir, &r.file, &t!("dev.noteEdited"))?;
                    self.saved.push(r.file.clone());
                }
                write_text(&path, &r.text)?;
                self.post(json!({"type": "saved", "file": r.file}));
                if r.file == p.entry_name() {
                    self.reload_meta();
                }
            }
            "apply" => {
                let p = self.project()?.clone();
                p.path(&r.file)?;
                let note = if r.note.is_empty() { t!("dev.noteApplied") } else { t!("dev.noteAppliedFrom", r.note) };
                versions::replace(&p.dir, &r.file, &r.text, &note)?;
                self.post(json!({"type": "applied", "file": r.file, "text": r.text}));
                self.post_versions(&r.file);
                self.post_files();
                if r.file == p.entry_name() {
                    self.reload_meta();
                }
            }
            "versions" => self.post_versions(&r.file),
            "version" => {
                let p = self.project()?;
                p.path(&r.file)?;
                let text = versions::read(&p.dir, &r.file, r.n)?;
                self.post(json!({"type": "version", "file": r.file, "n": r.n, "text": text}));
            }
            "restore" => {
                let p = self.project()?.clone();
                p.path(&r.file)?;
                let text = versions::restore(&p.dir, &r.file, r.n, &t!("dev.noteRestored", r.n))?;
                self.post(json!({"type": "applied", "file": r.file, "text": text}));
                self.post_versions(&r.file);
            }
            "run" => self.start_run(if r.debug { RunKind::Debug } else { RunKind::Run })?,
            "preview" => self.start_run(RunKind::Preview)?,
            "stop" => {
                if let Some(r) = &self.running {
                    r.run.kill();
                }
            }
            "intro" => {
                self.post(json!({"type": "copy", "what": "intro", "text": report::intro(self.project.as_ref())}));
            }
            "showFolder" => {
                let p = self.project()?;
                self.u.show_in_folder(&[p.entry.to_string_lossy().into_owned()])?;
            }
            "openUrl" => {
                if r.url.starts_with("https://") || r.url.starts_with("http://") {
                    self.u.open_path(&r.url)?;
                }
            }
            "chat" => {
                // The page chose a chat (or none: back to the start).
                if let Some(p) = &self.project {
                    let f = p.dir.join(project::DATA_DIR).join("dev.json");
                    let mut ps: ProjectSettings = load(&f);
                    if r.url.is_empty() {
                        ps.chat.remove(&p.entry_name());
                    } else {
                        ps.chat.insert(p.entry_name(), r.url.clone());
                    }
                    store(&f, &ps);
                }
                if r.url.is_empty() {
                    self.chat_url.clear();
                    self.w.browser(BrowserCommand::Navigate("about:blank".into()));
                    self.w.browser(BrowserCommand::Hide);
                } else {
                    self.w.browser(BrowserCommand::Navigate(r.url));
                }
            }
            "browser" => {
                let cmd = match r.path.as_str() {
                    "show" => BrowserCommand::Show { x: r.x, y: r.y, width: r.width, height: r.height, scale: if r.scale > 0.0 { r.scale } else { 1.0 } },
                    "hide" => BrowserCommand::Hide,
                    "back" => BrowserCommand::Back,
                    "forward" => BrowserCommand::Forward,
                    "reload" => BrowserCommand::Reload,
                    _ => return Ok(()),
                };
                self.w.browser(cmd);
            }
            _ => {}
        }
        Ok(())
    }

    /// The first message to the page: its texts and the choices.
    fn init(&self) -> Value {
        let gs: GlobalSettings = load(&self.o.settings);
        let recent: Vec<Value> = gs
            .recent
            .iter()
            .filter(|p| Path::new(p).is_file())
            .map(|p| json!({"path": p, "name": Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()}))
            .collect();
        json!({
            "type": "init",
            "lang": i18n::lang(),
            "texts": texts(),
            "services": SERVICES.iter().map(|(n, u)| json!({"name": n, "url": u})).collect::<Vec<_>>(),
            "recent": recent,
        })
    }

    fn open_dialog(&mut self, start: &str) {
        let paths = self.u.open_files(&PickerOptions {
            title: t!("cli.selectScript"),
            start_in: start.to_string(),
            filters: vec![FileFilter { description: t!("cli.scripts"), patterns: vec!["*.js".into(), "*.mjs".into(), "*.ts".into()] }],
            ..Default::default()
        });
        if let Some(p) = paths.into_iter().next()
            && let Err(e) = self.open(&p)
        {
            self.fail(&e);
        }
    }

    /// Opens a script: the page gets its files and the chat it was
    /// developed with.
    pub fn open(&mut self, path: &str) -> Result<()> {
        let p = Project::open(path)?;
        let text = read_text(&p.entry)?;
        let settings = p.dir.join(project::DATA_DIR).join("dev.json");
        let mut ps: ProjectSettings = load(&settings);
        let mut chat = ps.chat.get(&p.entry_name()).cloned().unwrap_or_default();
        // A script without a chat of its own takes the one open now.
        let keep_chat = chat.is_empty() && !self.chat_url.is_empty();
        if keep_chat {
            chat = self.chat_url.clone();
            ps.chat.insert(p.entry_name(), chat.clone());
            store(&settings, &ps);
        }
        // The recent scripts, newest first.
        let mut gs: GlobalSettings = load(&self.o.settings);
        let abs = p.entry.to_string_lossy().into_owned();
        gs.recent.retain(|r| r != &abs);
        gs.recent.insert(0, abs.clone());
        gs.recent.truncate(10);
        store(&self.o.settings, &gs);
        *self.dir.lock().unwrap() = Some(p.dir.clone());
        self.current = p.entry_name();
        self.saved.clear();
        self.post(json!({
            "type": "project",
            "path": abs,
            "dir": p.dir.to_string_lossy(),
            "entry": p.entry_name(),
            "text": text,
            "app": p.meta.is_app(),
            "html": p.meta.html,
            "title": p.meta.title,
            "files": p.files(),
            "chat": chat,
        }));
        let entry = p.entry_name();
        self.project = Some(p);
        self.post_versions(&entry);
        if chat.is_empty() {
            self.w.browser(BrowserCommand::Hide);
        } else if !keep_chat {
            self.w.browser(BrowserCommand::Navigate(chat));
        }
        Ok(())
    }

    fn reload_meta(&mut self) {
        let Some(p) = &self.project else { return };
        if let Ok(np) = Project::open(&p.entry.to_string_lossy()) {
            self.post(json!({"type": "meta", "app": np.meta.is_app(), "html": np.meta.html, "title": np.meta.title}));
            self.project = Some(np);
        }
    }

    fn post_files(&self) {
        if let Some(p) = &self.project {
            self.post(json!({"type": "files", "files": p.files()}));
        }
    }

    fn post_versions(&self, file: &str) {
        let Some(p) = &self.project else { return };
        let list = versions::list(&p.dir, file);
        self.post(json!({"type": "versions", "file": file, "versions": list}));
    }

    /// Creates a new script (and for an app its page) and opens it.
    fn create(&mut self, app: bool) -> Result<()> {
        let name = if app { t!("dev.newAppName") } else { t!("dev.newScriptName") };
        let Some(path) = self.u.save_file(&PickerOptions {
            title: t!("dev.newTitle"),
            suggested_name: format!("{name}.js"),
            filters: vec![FileFilter { description: t!("cli.scripts"), patterns: vec!["*.js".into(), "*.ts".into()] }],
            ..Default::default()
        }) else {
            return Ok(());
        };
        let p = Path::new(&path);
        let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if app {
            write_text(p, &format!(
                "// ==xljs==\n// @mode   app\n// @html   ui/index.html\n// @assets ui/\n// @title  {title}\n// ==/xljs==\n\n// {}\nexport function hello() {{\n  return \"{}\";\n}}\n",
                t!("dev.templateAppComment"),
                t!("dev.templateHello"),
            ))?;
            let html = p.parent().unwrap_or(Path::new(".")).join("ui").join("index.html");
            if !html.exists() {
                write_text(&html, &format!(
                    "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n</head>\n<body>\n  <h1>{title}</h1>\n  <button data-action=\"hello\" data-result=\"message\">OK</button>\n  <p data-bind=\"message\"></p>\n</body>\n</html>\n",
                    i18n::lang()
                ))?;
            }
        } else {
            write_text(p, &format!("// ==xljs==\n// @title {title}\n// ==/xljs==\n\nalert(\"{}\");\n", t!("dev.templateHello")))?;
        }
        self.open(&path)
    }

    fn browser_event(&mut self, b: BrowserEvent) {
        match b {
            BrowserEvent::Url(url) => {
                let web = url.starts_with("https://") || url.starts_with("http://");
                self.chat_url = if web { url.clone() } else { String::new() };
                if let Some(p) = &self.project
                    && (url.starts_with("https://") || url.starts_with("http://"))
                {
                    let f = p.dir.join(project::DATA_DIR).join("dev.json");
                    let mut ps: ProjectSettings = load(&f);
                    if ps.chat.get(&p.entry_name()) != Some(&url) {
                        ps.chat.insert(p.entry_name(), url.clone());
                        store(&f, &ps);
                    }
                }
                self.post(json!({"type": "url", "url": url}));
            }
            BrowserEvent::Downloaded { path, name } => {
                if let Err(e) = self.downloaded(&path, &name) {
                    self.fail(&e);
                }
            }
            BrowserEvent::DownloadFailed { name } => self.post(json!({"type": "error", "message": t!("dev.downloadFailed", name)})),
            BrowserEvent::Failed(e) => self.post(json!({"type": "browserFailed", "message": e})),
        }
    }

    /// A file from the chat: which file it replaces, and both texts.
    pub fn downloaded(&mut self, path: &str, name: &str) -> Result<()> {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let text = read_text(Path::new(path))?;
        if size > 4 << 20 || text.contains('\0') {
            self.post(json!({"type": "error", "message": t!("dev.downloadNotText", name)}));
            return Ok(());
        }
        let _ = std::fs::remove_file(path);
        let Some(p) = &self.project else {
            self.post(json!({"type": "error", "message": t!("dev.downloadNoScript", name)}));
            return Ok(());
        };
        let target = p.match_download(name, &self.current);
        let old = match &target {
            Some(t) => read_text(&p.path(t)?).ok(),
            None => None,
        };
        self.post(json!({
            "type": "download",
            "name": name,
            "text": text,
            "target": target,
            "old": old,
            "files": p.files(),
        }));
        Ok(())
    }

    fn start_run(&mut self, kind: RunKind) -> Result<()> {
        if self.running.as_ref().is_some_and(|r| r.run.running()) {
            return Err(err!("dev.alreadyRunning"));
        }
        let p = self.project()?.clone();
        let mut args = self.o.run_args.clone();
        if kind == RunKind::Debug {
            args.push("--debug".into());
        }
        args.push(p.entry.to_string_lossy().into_owned());
        let mut env = vec![(runner::DEV_ENV.to_string(), "1".to_string())];
        if std::env::var("XLJS_WATCHDOG").unwrap_or_default().is_empty() {
            env.push(("XLJS_WATCHDOG".into(), "on".into()));
        }
        if std::env::var("XLJS_STOP_KEY").unwrap_or_default().is_empty() {
            env.push(("XLJS_STOP_KEY".into(), "on".into()));
        }
        let preview = (kind == RunKind::Preview).then(|| std::env::temp_dir().join(format!("xljs-preview-{}-{}.json", std::process::id(), self.next_run)));
        if let Some(f) = &preview {
            let _ = std::fs::remove_file(f);
            env.push((runner::PREVIEW_ENV.into(), f.to_string_lossy().into_owned()));
        }
        let id = self.next_run;
        self.next_run += 1;
        let run = Run::start(&self.o.exe, &args, &p.dir, &env, id, self.tx.clone(), Event::Run)?;
        self.running = Some(Running { run, kind, stderr: String::new(), preview });
        self.post(json!({"type": "runStarted", "kind": format!("{kind:?}").to_lowercase()}));
        Ok(())
    }

    fn run_event(&mut self, e: RunEvent) {
        match e {
            RunEvent::Output { id, stderr, text } => {
                let Some(r) = &mut self.running else { return };
                if r.run.id != id {
                    return;
                }
                if stderr {
                    r.stderr.push_str(&text);
                    if r.stderr.len() > 1 << 20 {
                        let cut = r.stderr.len() - (1 << 19);
                        let cut = (cut..r.stderr.len()).find(|&i| r.stderr.is_char_boundary(i)).unwrap_or(0);
                        r.stderr.drain(..cut);
                    }
                }
                self.post(json!({"type": "output", "stream": if stderr { "stderr" } else { "stdout" }, "text": text}));
            }
            RunEvent::Exited { id, code } => {
                let Some(r) = self.running.take_if(|r| r.run.id == id) else { return };
                self.finished(r, code);
            }
        }
    }

    /// Reports the end of a run.
    fn finished(&mut self, r: Running, code: Option<i32>) {
        let Some(p) = self.project.clone() else { return };
        if let Some(f) = &r.preview {
            let form = std::fs::read_to_string(f).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let _ = std::fs::remove_file(f);
            match form {
                Some(form) => self.post(json!({"type": "previewForm", "form": form})),
                None => {
                    let message = if code == Some(0) { t!("dev.previewNoForm") } else { report::last_error(&r.stderr) };
                    self.post(json!({"type": "previewFailed", "message": message}));
                }
            }
            self.post(json!({"type": "runEnded", "kind": "preview", "code": code}));
            return;
        }
        let stopped = code == Some(crate::engine::watchdog::STOPPED_CODE);
        let failed = code != Some(0);
        let mut ev = json!({"type": "runEnded", "kind": format!("{:?}", r.kind).to_lowercase(), "code": code, "stopped": stopped});
        if failed {
            let message = report::last_error(&r.stderr);
            let loc = report::find_location(&message, &p.dir).or_else(|| report::find_location(&r.stderr, &p.dir));
            ev["message"] = json!(message);
            ev["location"] = json!(loc);
            ev["prompt"] = json!(report::prompt(&p, stopped, &message, loc.as_ref()));
        }
        self.post(ev);
    }

    /// Ends what is left when the window closes.
    pub fn shutdown(&mut self) {
        if let Some(r) = self.running.take() {
            r.run.kill();
        }
    }
}

/// The texts of the page (the keys under "devPage." without the prefix).
fn texts() -> Value {
    let m: serde_json::Map<String, Value> =
        i18n::messages().into_iter().filter_map(|(k, v)| k.strip_prefix("devPage.").map(|k| (k.to_string(), json!(v)))).collect();
    Value::Object(m)
}

#[cfg(test)]
mod tests;
