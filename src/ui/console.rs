use std::io::{BufRead, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use unicode_width::UnicodeWidthStr;

use super::*;
use crate::{Result, err, t};

/// Implements Ui on top of stdin/stderr.
pub struct Console {
    input: Mutex<Box<dyn BufRead + Send>>,
    out: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Whether input and output are terminals.
    in_tty: bool,
    out_tty: bool,
}

impl Console {
    /// A console UI bound to the process stdin/stderr.
    pub fn new() -> Console {
        Console {
            input: Mutex::new(Box::new(std::io::BufReader::new(std::io::stdin()))),
            out: Arc::new(Mutex::new(Box::new(std::io::stderr()))),
            in_tty: std::io::stdin().is_terminal(),
            out_tty: std::io::stderr().is_terminal(),
        }
    }

    /// A console UI reading input and writing to out (tests).
    pub fn with_io(input: impl BufRead + Send + 'static, out: impl Write + Send + 'static) -> Console {
        Console { input: Mutex::new(Box::new(input)), out: Arc::new(Mutex::new(Box::new(out))), in_tty: false, out_tty: false }
    }

    pub(crate) fn w(&self, s: &str) {
        let mut o = self.out.lock().unwrap();
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    }

    pub(crate) fn read_line(&self) -> Option<String> {
        let mut line = String::new();
        match self.input.lock().unwrap().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
        }
    }

    pub(crate) fn interactive(&self) -> bool {
        self.in_tty
    }

    fn header(title: &str) -> String {
        if title.is_empty() { String::new() } else { format!("[{title}] ") }
    }

    fn ask_path(&self, title: &str, message: &str) -> Vec<String> {
        let Some(line) = self.prompt(title, message, "") else { return Vec::new() };
        line.split(';').map(|p| p.trim().trim_matches('"').to_string()).filter(|p| !p.is_empty()).collect()
    }

    pub(crate) fn read_password(&self, label: &str, cur: &str) -> Option<Value> {
        if !self.in_tty {
            return self.prompt("", label, "").map(Value::String);
        }
        self.w(&format!("{label}: "));
        let r = rpassword::read_password();
        match r {
            Err(_) => None,
            Ok(s) if s.is_empty() => Some(Value::String(cur.to_string())),
            Ok(s) => Some(Value::String(s)),
        }
    }
}

impl Default for Console {
    fn default() -> Self {
        Console::new()
    }
}

pub(crate) fn filter_hint(fs: &[FileFilter]) -> String {
    let pats: Vec<&str> = fs.iter().flat_map(|f| f.patterns.iter().map(String::as_str)).collect();
    if pats.is_empty() { String::new() } else { format!(" ({})", pats.join(", ")) }
}

impl Ui for Console {
    fn name(&self) -> String {
        "console".into()
    }

    fn alert(&self, title: &str, message: &str) {
        self.w(&format!("{}{message}\n", Self::header(title)));
        if self.interactive() {
            self.w(&t!("ui.pressEnter"));
            self.read_line();
        }
    }

    fn confirm(&self, title: &str, message: &str) -> bool {
        self.w(&format!("{}{message} [y/N]: ", Self::header(title)));
        let Some(line) = self.read_line() else {
            self.w("\n");
            return false;
        };
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "はい" | "ok")
    }

    fn prompt(&self, title: &str, message: &str, def: &str) -> Option<String> {
        if def.is_empty() {
            self.w(&format!("{}{message}: ", Self::header(title)));
        } else {
            self.w(&format!("{}{message} [{def}]: ", Self::header(title)));
        }
        let Some(line) = self.read_line() else {
            self.w("\n");
            return None;
        };
        Some(if line.is_empty() { def.to_string() } else { line })
    }

    fn open_files(&self, o: &PickerOptions) -> Vec<String> {
        let mut msg = t!("ui.fileToOpen") + &filter_hint(&o.filters);
        if o.multiple {
            msg += &t!("ui.multipleHint");
        }
        let mut paths = self.ask_path(&o.title, &msg);
        if !o.multiple {
            paths.truncate(1);
        }
        paths
    }

    fn save_file(&self, o: &PickerOptions) -> Option<String> {
        let mut def = o.suggested_name.clone();
        if !def.is_empty() && !o.start_in.is_empty() {
            def = std::path::Path::new(&o.start_in).join(&def).to_string_lossy().into_owned();
        }
        let p = self.prompt(&o.title, &(t!("ui.saveAs") + &filter_hint(&o.filters)), &def)?;
        let p = p.trim().trim_matches('"').to_string();
        if p.is_empty() { None } else { Some(p) }
    }

    fn pick_directory(&self, o: &PickerOptions) -> Option<String> {
        self.ask_path(&o.title, &t!("ui.folder")).into_iter().next()
    }

    fn show_error(&self, _title: &str, message: &str) {
        self.w(&format!("{message}\n"));
    }

    fn notify(&self, title: &str, message: &str) {
        self.w(&format!("{}{message}\n", Self::header(title)));
    }

    fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress> {
        progress_opened();
        let p = ConsoleProgress {
            out: self.out.clone(),
            tty: self.out_tty,
            width: 80,
            st: Mutex::new(ProgressState {
                title: o.title.clone(),
                done: 0,
                total: o.total,
                message: o.message.clone(),
                last: None,
                last_pct: 0,
                closed: false,
                spinner: 0,
                rendered: false,
                last_line: String::new(),
            }),
        };
        p.render(&mut p.st.lock().unwrap(), true);
        Arc::new(p)
    }

    fn show_in_folder(&self, paths: &[String]) -> Result<()> {
        shell_show_in_folder(paths)
    }

    fn open_path(&self, target: &str) -> Result<()> {
        shell_open(target)
    }

    /// HTML windows need the native UI.
    fn open_window(&self, _o: WindowOptions) -> Result<Arc<dyn Window>> {
        Err(err!("ui.windowUnavailable"))
    }

    /// Forms and tables are asked on the console.
    fn supports_windows(&self) -> bool {
        false
    }

    fn form(&self, o: &FormOptions) -> Option<Map<String, Value>> {
        super::console_form::form(self, o)
    }

    fn table(&self, o: &TableOptions) -> Option<Vec<i64>> {
        super::console_form::table(self, o)
    }
}

struct ProgressState {
    title: String,
    done: i64,
    total: i64,
    message: String,
    last: Option<Instant>,
    last_pct: i64,
    closed: bool,
    spinner: usize,
    rendered: bool,
    last_line: String,
}

struct ConsoleProgress {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
    tty: bool,
    width: usize,
    st: Mutex<ProgressState>,
}

impl ConsoleProgress {
    fn write(&self, s: &str) {
        let mut o = self.out.lock().unwrap();
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    }

    fn render(&self, p: &mut ProgressState, force: bool) {
        if self.tty {
            if !force && p.last.is_some_and(|l| l.elapsed() < Duration::from_millis(100)) {
                return;
            }
            p.last = Some(Instant::now());
            let line = self.line(p);
            self.write(&format!("\r{line}\x1b[K"));
            p.rendered = true;
            return;
        }
        // Not a terminal: print a line every 10% (or at start/end).
        let pct = if p.total > 0 { p.done * 100 / p.total } else { -1 };
        if force || (pct >= 0 && pct / 10 != p.last_pct / 10) {
            p.last_pct = pct;
            let line = self.line(p);
            if line == p.last_line {
                return;
            }
            p.last_line = line.clone();
            self.write(&format!("{line}\n"));
            p.rendered = true;
        }
    }

    fn line(&self, p: &mut ProgressState) -> String {
        let mut b = String::new();
        if !p.title.is_empty() {
            b.push_str(&p.title);
            b.push(' ');
        }
        if p.total > 0 {
            const BAR: i64 = 20;
            let filled = (p.done * BAR / p.total).clamp(0, BAR);
            b.push_str(&format!(
                "[{}{}] {}/{} ({}%)",
                "#".repeat(filled as usize),
                "-".repeat((BAR - filled) as usize),
                p.done,
                p.total,
                p.done * 100 / p.total
            ));
        } else {
            let frames = ['|', '/', '-', '\\'];
            p.spinner = (p.spinner + 1) % frames.len();
            b.push_str(&format!("[{}] {}", frames[p.spinner], p.done));
        }
        if !p.message.is_empty() {
            b.push(' ');
            b.push_str(&p.message);
        }
        if self.tty {
            b = truncate_width(&b, self.width - 1);
        }
        b
    }
}

/// Cuts s to at most w display columns, ending with "…" when cut.
fn truncate_width(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut n = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if n + cw + 1 > w {
            break;
        }
        out.push(c);
        n += cw;
    }
    out + "…"
}

impl Progress for ConsoleProgress {
    fn update(&self, done: i64, total: i64, message: &str) {
        let mut p = self.st.lock().unwrap();
        if p.closed {
            return;
        }
        p.done = done;
        if total > 0 {
            p.total = total;
        }
        p.message = message.to_string();
        self.render(&mut p, false);
    }
    fn cancelled(&self) -> bool {
        interrupted()
    }
    fn close(&self) {
        let mut p = self.st.lock().unwrap();
        if p.closed {
            return;
        }
        self.render(&mut p, true);
        if self.tty && p.rendered {
            self.write("\n");
        }
        p.closed = true;
        progress_closed();
    }
}
