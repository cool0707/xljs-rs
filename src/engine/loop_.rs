//! A minimal event loop: timers plus callbacks posted from other threads.
//! Promise jobs run after every callback.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{Ctx, Function, Persistent, Value};

use crate::api::Host;

/// A callback posted to the script thread.
pub type Job = Box<dyn for<'js> FnOnce(&Ctx<'js>, &Host) -> rquickjs::Result<()> + Send>;

#[derive(Default)]
struct SharedState {
    posted: Vec<Job>,
    pending: i64,
}

/// The part of the loop other threads use (post, add_pending).
#[derive(Default)]
pub struct Shared {
    st: Mutex<SharedState>,
    cv: Condvar,
}

impl Shared {
    /// Schedules f to run on the loop thread. Safe from any thread; the
    /// caller must have called add_pending beforehand.
    pub fn post(&self, f: Job) {
        let mut st = self.st.lock().unwrap();
        st.posted.push(f);
        st.pending -= 1;
        self.cv.notify_all();
    }

    /// Keeps the loop running until a matching post.
    pub fn add_pending(&self) {
        self.st.lock().unwrap().pending += 1;
    }

    /// Whether nothing but n pending operations is left.
    pub fn idle(&self, n: i64) -> bool {
        let st = self.st.lock().unwrap();
        st.pending == n && st.posted.is_empty()
    }
}

struct Timer {
    seq: u64,
    when: Instant,
    interval: Duration,
    repeat: bool,
    func: Persistent<Function<'static>>,
    args: Vec<Persistent<Value<'static>>>,
}

/// The loop: lives on the script thread.
pub struct Loop {
    pub shared: Arc<Shared>,
    timers: RefCell<HashMap<i64, Timer>>,
    next_id: Cell<i64>,
    seq: Cell<u64>,
    /// Runs after each posted callback (see the debugger).
    pub after: RefCell<Option<Box<dyn Fn(&Host)>>>,
}

impl Loop {
    pub fn new() -> Loop {
        Loop {
            shared: Arc::new(Shared::default()),
            timers: RefCell::new(HashMap::new()),
            next_id: Cell::new(0),
            seq: Cell::new(0),
            after: RefCell::new(None),
        }
    }

    pub fn add_timer<'js>(&self, ctx: &Ctx<'js>, func: Function<'js>, delay: f64, repeat: bool, args: Vec<Value<'js>>) -> i64 {
        let delay = if delay.is_nan() || delay < 0.0 { 0.0 } else { delay };
        let mut d = Duration::from_secs_f64(delay / 1000.0);
        if repeat && d < Duration::from_millis(1) {
            d = Duration::from_millis(1);
        }
        self.next_id.set(self.next_id.get() + 1);
        self.seq.set(self.seq.get() + 1);
        let t = Timer {
            seq: self.seq.get(),
            when: Instant::now() + d,
            interval: d,
            repeat,
            func: Persistent::save(ctx, func),
            args: args.into_iter().map(|a| Persistent::save(ctx, a)).collect(),
        };
        self.timers.borrow_mut().insert(self.next_id.get(), t);
        self.next_id.get()
    }

    pub fn clear_timer(&self, id: i64) {
        self.timers.borrow_mut().remove(&id);
    }

    /// Drops everything that refers to JavaScript values (before the
    /// runtime goes away).
    pub fn clear(&self) {
        self.timers.borrow_mut().clear();
        self.shared.st.lock().unwrap().posted.clear();
    }

    fn next_timer(&self) -> Option<(i64, Instant)> {
        self.timers.borrow().iter().min_by(|a, b| (a.1.when, a.1.seq).cmp(&(b.1.when, b.1.seq))).map(|(id, t)| (*id, t.when))
    }

    /// Processes timers and posted callbacks until nothing is left or the
    /// script exits. Promise jobs run after each callback.
    pub fn run<'js>(&self, ctx: &Ctx<'js>, host: &Host) -> rquickjs::Result<()> {
        loop {
            drain_jobs(ctx);
            if host.exited() {
                return Ok(());
            }
            let (posted, pending) = {
                let mut st = self.shared.st.lock().unwrap();
                (std::mem::take(&mut st.posted), st.pending)
            };
            let had_posted = !posted.is_empty();
            if had_posted {
                // Something came from the UI: the script is responding.
                super::watchdog::waited();
            }
            for f in posted {
                f(ctx, host)?;
                drain_jobs(ctx);
                if host.exited() {
                    return Ok(());
                }
                if let Some(after) = &*self.after.borrow() {
                    after(host);
                }
            }
            if had_posted {
                continue;
            }
            let Some((id, when)) = self.next_timer() else {
                if pending == 0 {
                    return Ok(());
                }
                self.wait(None);
                super::watchdog::waited();
                continue;
            };
            let now = Instant::now();
            if when > now {
                let woken = if pending > 0 {
                    self.wait(Some(when - now))
                } else {
                    std::thread::sleep(when - now);
                    false
                };
                super::watchdog::waited();
                if woken {
                    continue;
                }
            }
            let (func, args) = {
                let mut timers = self.timers.borrow_mut();
                let Some(t) = timers.get_mut(&id) else { continue };
                let func = t.func.clone().restore(ctx)?;
                let args: Vec<Value> = t.args.iter().map(|a| a.clone().restore(ctx)).collect::<rquickjs::Result<_>>()?;
                if t.repeat {
                    self.seq.set(self.seq.get() + 1);
                    t.seq = self.seq.get();
                    t.when = Instant::now() + t.interval;
                } else {
                    timers.remove(&id);
                }
                (func, args)
            };
            let mut a = rquickjs::function::Args::new(ctx.clone(), args.len());
            for v in args {
                a.push_arg(v)?;
            }
            func.call_arg::<Value>(a).map(|_| ())?;
        }
    }

    /// Waits until something is posted (true) or the timeout passes.
    fn wait(&self, timeout: Option<Duration>) -> bool {
        let st = self.shared.st.lock().unwrap();
        if !st.posted.is_empty() {
            return true;
        }
        match timeout {
            None => {
                let st = self.shared.cv.wait_while(st, |s| s.posted.is_empty() && s.pending > 0).unwrap();
                !st.posted.is_empty()
            }
            Some(d) => {
                let (st, _) = self.shared.cv.wait_timeout_while(st, d, |s| s.posted.is_empty()).unwrap();
                !st.posted.is_empty()
            }
        }
    }
}

impl Default for Loop {
    fn default() -> Self {
        Loop::new()
    }
}

/// Runs the pending promise jobs.
pub fn drain_jobs(ctx: &Ctx) {
    while ctx.execute_pending_job() {}
}
