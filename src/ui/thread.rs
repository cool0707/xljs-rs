use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::*;

pub(crate) type Job = Box<dyn FnOnce() + Send>;

/// Runs functions on the UI thread.
pub(crate) trait UiThread: Send + Sync {
    /// Queues f to run on the UI thread. Safe from any thread.
    fn post(&self, f: Job);
    /// Runs queued functions (and, for the native UI, the window message
    /// loop) until done is set and the queue is empty.
    fn serve(&self, done: &Arc<(Mutex<bool>, Condvar)>);
    /// How long a progress cancellation state may be reused before asking
    /// the UI thread again (zero = always ask).
    fn cancel_poll(&self) -> Duration;
}

/// Runs script on a new thread while the calling thread serves UI
/// requests. For the native UI the caller must be the thread that created
/// it (the main thread). The UI handed to script forwards every call to the
/// calling thread, so dialogs and windows keep responding while the script
/// computes. It returns what script returns.
pub fn run<T: Send + 'static>(u: Arc<dyn Ui>, script: impl FnOnce(Arc<dyn Ui>) -> T + Send + 'static) -> T {
    let t = new_thread(&u);
    let done = Arc::new((Mutex::new(false), Condvar::new()));
    let proxy: Arc<dyn Ui> = Arc::new(Threaded { u, t: t.clone() });
    let (tx, rx) = mpsc::channel();
    let (d, t2) = (done.clone(), t.clone());
    let handle = std::thread::Builder::new()
        .name("script".into())
        .stack_size(64 << 20)
        .spawn(move || {
            // Mark the script as done even when it panics.
            struct Done(Arc<(Mutex<bool>, Condvar)>, Arc<dyn UiThread>);
            impl Drop for Done {
                fn drop(&mut self) {
                    *self.0.0.lock().unwrap() = true;
                    self.0.1.notify_all();
                    self.1.post(Box::new(|| {})); // wake serve
                }
            }
            let _done = Done(d, t2);
            let _ = tx.send(script(proxy));
        })
        .expect("cannot start the script thread");
    t.serve(&done);
    match rx.recv() {
        Ok(v) => {
            let _ = handle.join();
            v
        }
        Err(_) => match handle.join() {
            Err(p) => std::panic::resume_unwind(p),
            Ok(()) => unreachable!("the script thread ended without a result"),
        },
    }
}

#[cfg(not(windows))]
fn new_thread(_: &Arc<dyn Ui>) -> Arc<dyn UiThread> {
    Arc::new(ChanThread::default())
}

#[cfg(windows)]
fn new_thread(u: &Arc<dyn Ui>) -> Arc<dyn UiThread> {
    if u.name() == "native" {
        return super::native::new_ui_thread();
    }
    Arc::new(ChanThread::default())
}

/// The UI thread for UIs without a message loop (console, mock): the
/// calling thread runs queued functions.
#[derive(Default)]
pub(crate) struct ChanThread {
    q: Mutex<VecDeque<Job>>,
    cv: Condvar,
}

impl UiThread for ChanThread {
    fn post(&self, f: Job) {
        self.q.lock().unwrap().push_back(f);
        self.cv.notify_all();
    }

    fn serve(&self, done: &Arc<(Mutex<bool>, Condvar)>) {
        loop {
            let job = {
                let mut q = self.q.lock().unwrap();
                loop {
                    if let Some(j) = q.pop_front() {
                        break Some(j);
                    }
                    if *done.0.lock().unwrap() {
                        break None;
                    }
                    q = self.cv.wait_timeout(q, Duration::from_millis(50)).unwrap().0;
                }
            };
            match job {
                Some(j) => j(),
                None => return,
            }
        }
    }

    fn cancel_poll(&self) -> Duration {
        Duration::ZERO
    }
}

/// Runs f on the UI thread and waits for its result.
pub(crate) fn call<T: Send + 'static>(t: &Arc<dyn UiThread>, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::sync_channel(1);
    t.post(Box::new(move || {
        let _ = tx.send(f());
    }));
    let v = rx.recv().expect("the UI thread stopped");
    // The script waited for the UI: it is not stuck in a loop.
    crate::engine::watchdog::waited();
    v
}

/// Forwards UI calls to the UI thread.
struct Threaded {
    u: Arc<dyn Ui>,
    t: Arc<dyn UiThread>,
}

impl Threaded {
    fn on<T: Send + 'static>(&self, f: impl FnOnce(&dyn Ui) -> T + Send + 'static) -> T {
        let u = self.u.clone();
        call(&self.t, move || f(&*u))
    }
}

impl Ui for Threaded {
    fn name(&self) -> String {
        self.u.name()
    }
    fn alert(&self, title: &str, message: &str) {
        let (a, b) = (title.to_string(), message.to_string());
        self.on(move |u| u.alert(&a, &b))
    }
    fn confirm(&self, title: &str, message: &str) -> bool {
        let (a, b) = (title.to_string(), message.to_string());
        self.on(move |u| u.confirm(&a, &b))
    }
    fn prompt(&self, title: &str, message: &str, def: &str) -> Option<String> {
        let (a, b, c) = (title.to_string(), message.to_string(), def.to_string());
        self.on(move |u| u.prompt(&a, &b, &c))
    }
    fn open_files(&self, o: &PickerOptions) -> Vec<String> {
        let o = o.clone();
        self.on(move |u| u.open_files(&o))
    }
    fn save_file(&self, o: &PickerOptions) -> Option<String> {
        let o = o.clone();
        self.on(move |u| u.save_file(&o))
    }
    fn pick_directory(&self, o: &PickerOptions) -> Option<String> {
        let o = o.clone();
        self.on(move |u| u.pick_directory(&o))
    }
    fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress> {
        let o = o.clone();
        let inner = self.on(move |u| u.progress(&o));
        Arc::new(ThreadedProgress { p: inner, t: self.t.clone(), state: Mutex::new((None, false)) })
    }
    fn show_error(&self, title: &str, message: &str) {
        let (a, b) = (title.to_string(), message.to_string());
        self.on(move |u| u.show_error(&a, &b))
    }
    fn show_in_folder(&self, paths: &[String]) -> Result<()> {
        let p = paths.to_vec();
        self.on(move |u| u.show_in_folder(&p))
    }
    fn open_path(&self, target: &str) -> Result<()> {
        let t = target.to_string();
        self.on(move |u| u.open_path(&t))
    }
    fn open_window(&self, o: WindowOptions) -> Result<Arc<dyn Window>> {
        let w = self.on(move |u| u.open_window(o))?;
        Ok(Arc::new(ThreadedWindow { w, t: self.t.clone() }))
    }
    fn supports_windows(&self) -> bool {
        self.u.supports_windows()
    }
    fn form(&self, o: &FormOptions) -> Option<Map<String, Value>> {
        let o = o.clone();
        self.on(move |u| u.form(&o))
    }
    fn table(&self, o: &TableOptions) -> Option<Vec<i64>> {
        let o = o.clone();
        self.on(move |u| u.table(&o))
    }
    fn choose(&self, title: &str, message: &str, choices: &[Choice]) -> Option<i64> {
        let (a, b, c) = (title.to_string(), message.to_string(), choices.to_vec());
        self.on(move |u| u.choose(&a, &b, &c))
    }
    fn notify(&self, title: &str, message: &str) {
        // Does not wait, like progress updates.
        let (u, a, b) = (self.u.clone(), title.to_string(), message.to_string());
        self.t.post(Box::new(move || u.notify(&a, &b)));
    }
    fn print_pdf(&self, job: PdfJob, done: PdfDone) {
        // done comes later, from the UI thread.
        let u = self.u.clone();
        self.t.post(Box::new(move || u.print_pdf(job, done)));
    }
}

/// Forwards progress calls. Updates do not wait; the cancellation state is
/// polled at most every cancel_poll.
struct ThreadedProgress {
    p: Arc<dyn Progress>,
    t: Arc<dyn UiThread>,
    state: Mutex<(Option<Instant>, bool)>, // last check, cancelled
}

impl Progress for ThreadedProgress {
    fn update(&self, done: i64, total: i64, message: &str) {
        let (p, m) = (self.p.clone(), message.to_string());
        self.t.post(Box::new(move || p.update(done, total, &m)));
    }
    fn cancelled(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        if st.1 {
            return true;
        }
        let d = self.t.cancel_poll();
        if !d.is_zero()
            && let Some(at) = st.0
            && at.elapsed() < d
        {
            return false;
        }
        let p = self.p.clone();
        st.1 = call(&self.t, move || p.cancelled());
        st.0 = Some(Instant::now());
        st.1
    }
    fn close(&self) {
        let p = self.p.clone();
        call(&self.t, move || p.close())
    }
}

/// Forwards window calls. Messages do not wait.
struct ThreadedWindow {
    w: Arc<dyn Window>,
    t: Arc<dyn UiThread>,
}

impl Window for ThreadedWindow {
    fn post(&self, msg: String) {
        let w = self.w.clone();
        self.t.post(Box::new(move || w.post(msg)));
    }
    fn close(&self) {
        let w = self.w.clone();
        call(&self.t, move || w.close())
    }
    fn browser(&self, cmd: BrowserCommand) {
        let w = self.w.clone();
        self.t.post(Box::new(move || w.browser(cmd)));
    }
}
