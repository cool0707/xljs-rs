//! Stops a script stuck in an endless loop. When JavaScript runs longer
//! than the limit without waiting (for a dialog, a window, a timer or a
//! posted callback), the user is asked whether to stop it. The script is
//! paused while the question is open; "stop" ends it like exit(130).
//!
//! The waits mark themselves with `waited()` (the event loop and the UI
//! calls of ui::run); the check runs in the QuickJS interrupt handler.

use std::cell::Cell;
use std::rc::Weak;
use std::time::{Duration, Instant};

use crate::api::Host;
use crate::t;

/// The exit code of a script stopped by the user (like Ctrl+C).
pub const STOPPED_CODE: i32 = 130;

/// The limit the CLI uses when nothing else is given.
pub const DEFAULT_LIMIT: Duration = Duration::from_secs(15);

thread_local! {
    /// When the current thread last stopped waiting.
    static ACTIVE_SINCE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Records that the current thread has just waited (the script is not
/// stuck). Cheap; safe on any thread.
pub(crate) fn waited() {
    ACTIVE_SINCE.with(|c| c.set(Some(Instant::now())));
}

fn busy_for() -> Duration {
    ACTIVE_SINCE.with(|c| c.get().map(|t| t.elapsed()).unwrap_or_default())
}

/// The interrupt handler for a script run on this thread with the limit.
pub(crate) fn handler(host: Weak<Host>, limit: Duration) -> rquickjs::runtime::InterruptHandler {
    waited();
    let mut stopped = false;
    Box::new(move || {
        if stopped {
            return true;
        }
        if busy_for() < limit {
            return false;
        }
        let Some(h) = host.upgrade() else { return false };
        let stop = h.env.ui.confirm(&h.env.title, &t!("engine.stuckConfirm", limit.as_secs_f64()));
        waited();
        if stop {
            stopped = true;
            let _ = writeln!(h.env.stderr.lock().unwrap(), "{}", t!("engine.stopped"));
            h.set_exit(STOPPED_CODE);
        }
        stop
    })
}
