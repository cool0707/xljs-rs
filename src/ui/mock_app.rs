use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::mock::{LogSink, log_to};
use super::{Window, WindowOptions, quote};

/// One action of a mock window scenario (the "app" answers): the mock
/// plays the page and talks to the script with the messages of the page
/// runtime (web/xljs.js). Exactly one of the action fields is set.
///
/// ```json
/// {"invoke": "aggregate", "args": ["2026-04"], "expect": {"ok": true}}
/// {"emit": "progress", "args": [50]}
/// {"state": {"month": "2026-04"}}
/// {"event": "click", "selector": "#run", "detail": {"value": "x"}}
/// {"handle": "render", "result": 3}   // answer to win.invoke("render")
/// {"wait": 100}                       // milliseconds
/// {"close": true}
/// ```
///
/// The window closes after the last step. Arguments are in the wire format
/// ({"$file": path}, {"$date": iso}, ...).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppStep {
    pub invoke: String,
    pub args: Vec<Value>,
    pub expect: Option<Value>,
    pub emit: String,
    pub state: Option<Map<String, Value>>,
    pub event: String,
    pub selector: String,
    pub detail: Map<String, Value>,
    pub handle: String,
    pub result: Option<Value>,
    pub wait: u64,
    pub close: bool,
}

/// How long a step waits for the script.
const APP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct DomHandler {
    hid: i64,
    sel: String,
    kind: String,
}

#[derive(Default)]
struct State {
    closed: bool,
    next_id: i64,
    waiting: HashMap<i64, mpsc::Sender<Value>>,
    dom: HashMap<i64, DomHandler>,
    replies: HashMap<String, Value>,
}

/// A mock window that plays a scenario.
pub(crate) struct AppWindow {
    log: Option<LogSink>,
    o: WindowOptions,
    st: Mutex<State>,
    inited: AtomicBool,
    done: AtomicBool,
}

fn raw(v: &Value) -> String {
    v.to_string()
}

fn raw_list(vs: &[Value]) -> String {
    vs.iter().map(raw).collect::<Vec<_>>().join(", ")
}

fn gs(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn gi(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

impl AppWindow {
    pub(crate) fn start(log: Option<LogSink>, o: WindowOptions, steps: Vec<AppStep>) -> Arc<dyn Window> {
        let w = Arc::new(AppWindow { log, o, st: Mutex::new(State::default()), inited: AtomicBool::new(false), done: AtomicBool::new(false) });
        let w2 = w.clone();
        std::thread::spawn(move || w2.play(steps));
        w
    }

    fn logf(&self, s: &str) {
        log_to(&self.log, s);
    }

    fn send(&self, m: Value) {
        if let Some(f) = &self.o.on_message {
            f(m.to_string(), Vec::new());
        }
    }

    fn closed(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// Waits until cond holds, the window closes or the timeout passes; it
    /// returns whether cond held.
    fn wait_for(&self, timeout: Duration, cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if cond() {
                return true;
            }
            if self.closed() || Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn play(self: Arc<Self>, steps: Vec<AppStep>) {
        if self.o.on_message.is_none() {
            self.close();
            return;
        }
        self.send(json!({"xljs": "ready"}));
        if !self.wait_for(APP_TIMEOUT, || self.inited.load(Ordering::SeqCst)) {
            if self.closed() {
                return;
            }
            self.logf("app: no answer to ready");
        }
        for s in steps {
            if self.closed() {
                return;
            }
            if !s.invoke.is_empty() {
                self.invoke(&s);
            } else if !s.emit.is_empty() {
                self.logf(&format!("app: emit({}, {})", quote(&s.emit), raw_list(&s.args)));
                self.send(json!({"xljs": "event", "name": s.emit, "args": s.args}));
            } else if let Some(state) = &s.state {
                let mut keys: Vec<&String> = state.keys().collect();
                keys.sort();
                for k in keys {
                    let v = super::mock::sorted_json(&state[k]);
                    self.logf(&format!("app: state.{k} = {v}"));
                    self.send(json!({"xljs": "state", "key": k, "value": state[k]}));
                }
            } else if !s.event.is_empty() {
                self.fire(&s);
            } else if !s.handle.is_empty() {
                self.st.lock().unwrap().replies.insert(s.handle.clone(), s.result.clone().unwrap_or(Value::Null));
            } else if s.wait > 0 {
                std::thread::sleep(Duration::from_millis(s.wait));
            } else if s.close {
                self.close();
                return;
            }
        }
        self.close();
    }

    fn invoke(&self, s: &AppStep) {
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut st = self.st.lock().unwrap();
            st.next_id += 1;
            let id = st.next_id;
            st.waiting.insert(id, tx);
            st.next_id
        };
        self.send(json!({"xljs": "invoke", "id": id, "name": s.invoke, "args": s.args}));
        let call = format!("app: invoke {}({})", s.invoke, raw_list(&s.args));
        let deadline = Instant::now() + APP_TIMEOUT;
        loop {
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(r) => {
                    if gs(&r, "xljs") == "error" {
                        let e = r.get("error").cloned().unwrap_or(Value::Null);
                        self.logf(&format!("{call} -> error: {}: {}", gs(&e, "name"), gs(&e, "message")));
                        return;
                    }
                    let v = r.get("value").map(raw).unwrap_or_else(|| "undefined".into());
                    self.logf(&format!("{call} -> {v}"));
                    if let Some(want) = &s.expect
                        && r.get("value") != Some(want)
                    {
                        self.logf(&format!("app: invoke {}: expected {}", s.invoke, raw(want)));
                    }
                    return;
                }
                Err(mpsc::RecvTimeoutError::Timeout) if !self.closed() && Instant::now() < deadline => {}
                Err(_) => {
                    if !self.closed() {
                        self.logf(&format!("{call} -> timeout"));
                    }
                    return;
                }
            }
        }
    }

    /// Sends a DOM event to the handlers registered (win.$(sel).on) for the
    /// step's selector and type, waiting a little for late registrations.
    fn fire(&self, s: &AppStep) {
        let deadline = Instant::now() + APP_TIMEOUT;
        loop {
            let mut hids: Vec<i64> = {
                let st = self.st.lock().unwrap();
                st.dom.values().filter(|h| h.sel == s.selector && h.kind == s.event).map(|h| h.hid).collect()
            };
            if !hids.is_empty() {
                hids.sort();
                self.logf(&format!("app: {} {}", s.event, s.selector));
                for hid in hids {
                    let mut ev = Map::new();
                    ev.insert("type".into(), json!(s.event));
                    ev.insert("id".into(), json!(""));
                    ev.insert("name".into(), json!(""));
                    ev.insert("dataset".into(), json!({}));
                    for (k, v) in &s.detail {
                        ev.insert(k.clone(), v.clone());
                    }
                    self.send(json!({"xljs": "dom-event", "hid": hid, "event": ev}));
                }
                return;
            }
            if Instant::now() > deadline {
                self.logf(&format!("app: no {} handler for {}", s.event, s.selector));
                return;
            }
            if self.closed() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Window for AppWindow {
    /// Receives a message from the script: it logs it and answers the
    /// requests (reads, evaluate and invoke get null unless a handle step
    /// set a result).
    fn post(&self, msg: String) {
        let m: Value = match serde_json::from_str(&msg) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                self.logf(&format!("window.post({})", quote(&msg)));
                return;
            }
        };
        let kind = gs(&m, "xljs");
        let reply = {
            let mut st = self.st.lock().unwrap();
            if st.closed {
                return;
            }
            match kind.as_str() {
                "init" => {
                    if let Some(hs) = m.get("handlers").and_then(Value::as_array) {
                        for h in hs {
                            st.dom.insert(gi(h, "hid"), DomHandler { hid: gi(h, "hid"), sel: gs(h, "sel"), kind: gs(h, "type") });
                        }
                    }
                    self.inited.store(true, Ordering::SeqCst);
                }
                "result" | "error" => {
                    if let Some(tx) = st.waiting.remove(&gi(&m, "id")) {
                        let _ = tx.send(m.clone());
                    }
                }
                "dom" => match gs(&m, "op").as_str() {
                    "on" => {
                        st.dom.insert(gi(&m, "hid"), DomHandler { hid: gi(&m, "hid"), sel: gs(&m, "sel"), kind: gs(&m, "type") });
                    }
                    "off" => {
                        st.dom.remove(&gi(&m, "hid"));
                    }
                    _ => {}
                },
                _ => {}
            }
            st.replies.get(&gs(&m, "name")).cloned()
        };
        let args: Vec<Value> = m.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
        let id = gi(&m, "id");
        match kind.as_str() {
            "init" => self.logf(&format!("window.init({})", m.get("state").map(raw).unwrap_or_else(|| "undefined".into()))),
            "state" => {
                if m.get("deleted").and_then(Value::as_bool).unwrap_or(false) {
                    self.logf(&format!("window.state.{} deleted", gs(&m, "key")));
                } else {
                    let v = m.get("value").map(raw).unwrap_or_else(|| "undefined".into());
                    self.logf(&format!("window.state.{} = {v}", gs(&m, "key")));
                }
            }
            "event" => self.logf(&format!("window.emit({}, {})", quote(&gs(&m, "name")), raw_list(&args))),
            "dom" => {
                let (sel, op) = (gs(&m, "sel"), gs(&m, "op"));
                if op == "on" || op == "off" {
                    self.logf(&format!("window.$({}).{op}({})", quote(&sel), quote(&gs(&m, "type"))));
                } else if id != 0 {
                    self.logf(&format!("window.$({}).{op}({}) -> null", quote(&sel), raw_list(&args)));
                    self.send(json!({"xljs": "result", "id": id, "value": null}));
                } else {
                    self.logf(&format!("window.$({}).{op}({})", quote(&sel), raw_list(&args)));
                }
            }
            "invoke" => {
                let reply = reply.unwrap_or(Value::Null);
                self.logf(&format!("window.invoke({}, {}) -> {}", quote(&gs(&m, "name")), raw_list(&args), raw(&reply)));
                self.send(json!({"xljs": "result", "id": id, "value": reply}));
            }
            "eval" => {
                self.logf(&format!("window.evaluate({}) -> null", quote(&first_line(&gs(&m, "code")))));
                self.send(json!({"xljs": "result", "id": id, "value": null}));
            }
            _ => {}
        }
    }

    fn close(&self) {
        {
            let mut st = self.st.lock().unwrap();
            if st.closed {
                return;
            }
            st.closed = true;
            self.done.store(true, Ordering::SeqCst);
        }
        self.logf(&format!("window.closed({})", quote(&self.o.title)));
        if let Some(f) = &self.o.on_closed {
            f();
        }
    }
}

fn first_line(s: &str) -> String {
    match s.find('\n') {
        Some(i) => format!("{} ...", &s[..i]),
        None => s.to_string(),
    }
}
