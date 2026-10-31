use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::*;
use crate::{Error, Result};

/// Where the mock writes its log.
pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Answers UI requests from a prepared queue and logs every call. It is
/// meant for unattended runs and automated tests.
///
/// Answers file format (all keys optional):
///
/// ```json
/// {
///   "confirm": [true, false],
///   "prompt": ["text", null],
///   "showOpenFilePicker": [["a.xlsx", "b.xlsx"], []],
///   "showSaveFilePicker": ["out.xlsx", null],
///   "showDirectoryPicker": ["data", null],
///   "progress": [{"cancelAfter": 3}],
///   "form": [{"src": "data", "tax": true}, null],
///   "table": [[0, 2], null],
///   "app": [[{"invoke": "aggregate", "args": ["2026-04"]}, {"close": true}]],
///   "choose": [0, null]
/// }
/// ```
///
/// When a queue is empty the call behaves as if the user cancelled. A form
/// answer lists the values that differ from the current ones; a table
/// answer lists the indices of the chosen rows and a choose answer the
/// index of the chosen option (the start screen of xljs). Each window plays
/// the next "app" scenario (see AppStep) and then closes; without one it is
/// logged and closed immediately.
pub struct Mock {
    log: Option<LogSink>,
    answers: Mutex<HashMap<String, VecDeque<Value>>>,
}

impl Mock {
    /// A mock UI with the answers in the file at answers_path (may be "").
    pub fn new(answers_path: &str, log: Option<LogSink>) -> Result<Mock> {
        if answers_path.is_empty() {
            return Mock::from_json("", log);
        }
        let data = std::fs::read_to_string(answers_path).map_err(|e| Error::path("open", answers_path, e))?;
        Mock::from_json(&data, log).map_err(|e| e.context(answers_path))
    }

    /// A mock UI from an answers JSON document.
    pub fn from_json(doc: &str, log: Option<LogSink>) -> Result<Mock> {
        let mut answers = HashMap::new();
        if !doc.trim().is_empty() {
            let m: HashMap<String, Vec<Value>> = serde_json::from_str(doc)?;
            answers = m.into_iter().map(|(k, v)| (k, v.into())).collect();
        }
        Ok(Mock { log, answers: Mutex::new(answers) })
    }

    /// A sink that writes to stderr.
    pub fn stderr_log() -> LogSink {
        Arc::new(|s: &str| eprintln!("{s}"))
    }

    fn next_raw(&self, key: &str) -> Option<Value> {
        let mut a = self.answers.lock().unwrap();
        let v = a.get_mut(key)?.pop_front()?;
        if v.is_null() { None } else { Some(v) }
    }

    pub(crate) fn next<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let v = self.next_raw(key)?;
        match serde_json::from_value(v.clone()) {
            Ok(x) => Some(x),
            Err(e) => {
                self.logf(&format!("{key}: invalid answer {v}: {e}"));
                None
            }
        }
    }

    pub(crate) fn logf(&self, s: &str) {
        if let Some(l) = &self.log {
            l(&format!("[mock] {s}"));
        }
    }
}

impl Ui for Mock {
    fn name(&self) -> String {
        "mock".into()
    }

    fn alert(&self, _title: &str, message: &str) {
        self.logf(&format!("alert({})", quote(message)));
    }

    fn confirm(&self, _title: &str, message: &str) -> bool {
        let v: bool = self.next("confirm").unwrap_or(false);
        self.logf(&format!("confirm({}) -> {v}", quote(message)));
        v
    }

    fn choose(&self, _title: &str, message: &str, choices: &[Choice]) -> Option<i64> {
        let mut v: i64 = self.next("choose").unwrap_or(-1);
        if v < 0 || v >= choices.len() as i64 {
            v = -1;
        }
        self.logf(&format!("choose({}) -> {v}", quote(message)));
        Some(v)
    }

    fn prompt(&self, _title: &str, message: &str, _def: &str) -> Option<String> {
        let v: Option<String> = self.next("prompt");
        match &v {
            Some(s) => self.logf(&format!("prompt({}) -> {}", quote(message), quote(s))),
            None => self.logf(&format!("prompt({}) -> null", quote(message))),
        }
        v
    }

    fn open_files(&self, o: &PickerOptions) -> Vec<String> {
        let mut v: Vec<String> = self.next("showOpenFilePicker").unwrap_or_default();
        if !o.multiple && v.len() > 1 {
            v.truncate(1);
        }
        self.logf(&format!("showOpenFilePicker() -> {}", quote_list(&v)));
        v
    }

    fn save_file(&self, o: &PickerOptions) -> Option<String> {
        let v: Option<String> = self.next("showSaveFilePicker");
        self.logf(&format!("showSaveFilePicker({}) -> {}", quote(&o.suggested_name), quote(v.as_deref().unwrap_or(""))));
        v.filter(|s| !s.is_empty())
    }

    fn pick_directory(&self, _o: &PickerOptions) -> Option<String> {
        let v: Option<String> = self.next("showDirectoryPicker");
        self.logf(&format!("showDirectoryPicker() -> {}", quote(v.as_deref().unwrap_or(""))));
        v.filter(|s| !s.is_empty())
    }

    fn show_error(&self, _title: &str, message: &str) {
        self.logf(&format!("error: {message}"));
    }

    fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress> {
        #[derive(serde::Deserialize, Default)]
        struct Cfg {
            #[serde(default, rename = "cancelAfter")]
            cancel_after: i64,
        }
        let cfg: Cfg = self.next("progress").unwrap_or_default();
        self.logf(&format!("progress({}, total={}) start", quote(&o.title), o.total));
        progress_opened();
        Arc::new(MockProgress {
            log: self.log.clone(),
            cancel_after: cfg.cancel_after,
            st: Mutex::new(MockProgressState { updates: 0, done: 0, total: o.total, closed: false }),
        })
    }

    fn show_in_folder(&self, paths: &[String]) -> Result<()> {
        self.logf(&format!("showItemInFolder({})", quote_list(paths)));
        super::shell::check_paths(paths)
    }

    fn open_path(&self, target: &str) -> Result<()> {
        self.logf(&format!("openPath({})", quote(target)));
        Ok(())
    }

    /// Forms and tables are answered from the queue.
    fn supports_windows(&self) -> bool {
        false
    }

    fn form(&self, o: &FormOptions) -> Option<Map<String, Value>> {
        if !o.error.is_empty() {
            self.logf(&format!("form error: {}", o.error));
        }
        for f in &o.fields {
            if !f.error.is_empty() {
                self.logf(&format!("form error: {}: {}", f.name, f.error));
            }
            for e in &f.cell_errors {
                self.logf(&format!("form error: {}: {}", f.name, super::console_form::cell_error_text(f, e)));
            }
        }
        let Some(v) = self.next::<Map<String, Value>>("form") else {
            self.logf(&format!("form({}) -> null", quote(&o.title)));
            return None;
        };
        self.logf(&format!("form({}) -> {}", quote(&o.title), sorted_json(&Value::Object(v.clone()))));
        Some(v)
    }

    fn table(&self, o: &TableOptions) -> Option<Vec<i64>> {
        self.logf(&format!("table({}, {} rows, {})", quote(&o.title), o.rows.len(), quote_list(&o.columns)));
        if o.select == SELECT_NONE {
            return Some(Vec::new());
        }
        let Some(mut v) = self.next::<Vec<i64>>("table") else {
            self.logf("table -> cancel");
            return None;
        };
        if o.select == SELECT_SINGLE && v.len() > 1 {
            v.truncate(1);
        }
        let items: Vec<String> = v.iter().map(|i| i.to_string()).collect();
        self.logf(&format!("table -> [{}]", items.join(" ")));
        Some(v)
    }

    /// Logs the window and closes it right away, as if the user had closed
    /// it without doing anything (unless an "app" scenario is queued).
    fn open_window(&self, o: WindowOptions) -> Result<Arc<dyn Window>> {
        self.logf(&format!(
            "window.open({}, {}, {}x{}{})",
            quote(&o.title),
            quote(&o.url),
            o.width,
            o.height,
            if o.fixed_size { ", fixed" } else { "" }
        ));
        if !o.icon.is_empty() {
            self.logf(&format!("window.icon({} bytes)", o.icon.len()));
        }
        if let Some(steps) = self.next::<Vec<AppStep>>("app") {
            return Ok(super::mock_app::AppWindow::start(self.log.clone(), o, steps));
        }
        let w = Arc::new(MockWindow { log: self.log.clone(), o, closed: Mutex::new(false) });
        w.close();
        Ok(w)
    }
}

/// JSON with the keys of objects sorted (like Go's encoding of maps).
pub(crate) fn sorted_json(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                Value::Object(keys.into_iter().map(|k| (k.clone(), sort(&m[k]))).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            x => x.clone(),
        }
    }
    sort(v).to_string()
}

pub(crate) fn log_to(log: &Option<LogSink>, s: &str) {
    if let Some(l) = log {
        l(&format!("[mock] {s}"));
    }
}

struct MockProgressState {
    updates: i64,
    done: i64,
    total: i64,
    closed: bool,
}

struct MockProgress {
    log: Option<LogSink>,
    cancel_after: i64,
    st: Mutex<MockProgressState>,
}

impl Progress for MockProgress {
    fn update(&self, done: i64, total: i64, _message: &str) {
        let mut s = self.st.lock().unwrap();
        s.updates += 1;
        s.done = done;
        if total > 0 {
            s.total = total;
        }
    }
    fn cancelled(&self) -> bool {
        let s = self.st.lock().unwrap();
        (self.cancel_after > 0 && s.updates >= self.cancel_after) || interrupted()
    }
    fn close(&self) {
        let mut s = self.st.lock().unwrap();
        if s.closed {
            return;
        }
        s.closed = true;
        progress_closed();
        log_to(&self.log, &format!("progress closed at {}/{}", s.done, s.total));
    }
}

struct MockWindow {
    log: Option<LogSink>,
    o: WindowOptions,
    closed: Mutex<bool>,
}

impl Window for MockWindow {
    fn post(&self, msg: String) {
        if !*self.closed.lock().unwrap() {
            log_to(&self.log, &format!("window.post({})", quote(&msg)));
        }
    }
    fn close(&self) {
        {
            let mut c = self.closed.lock().unwrap();
            if *c {
                return;
            }
            *c = true;
        }
        log_to(&self.log, &format!("window.closed({})", quote(&self.o.title)));
        if let Some(f) = &self.o.on_closed {
            f();
        }
    }
}
