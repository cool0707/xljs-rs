//! Stopping a running script by the user, in two ways:
//!
//! - the watchdog: when JavaScript runs longer than the limit without
//!   waiting (for a dialog, a window, a timer or a posted callback), the
//!   user is asked whether to stop it (an endless loop);
//! - the stop key (Esc by default): pressing it while the script runs
//!   JavaScript asks the same.
//!
//! The script is paused while the question is open; "stop" ends it like
//! exit(130) with an uncatchable exception. The waits mark themselves with
//! `waited()` (the event loop and the UI calls of ui::run); the checks run
//! in the QuickJS interrupt handler on the script thread. The key is read
//! by a thread of its own (GetAsyncKeyState, so it works without a window
//! of the script having the focus; Windows only).

use std::cell::RefCell;
use std::rc::Weak;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::api::Host;
use crate::t;

/// The exit code of a script stopped by the user (like Ctrl+C).
pub const STOPPED_CODE: i32 = 130;

/// The limit the CLI uses when nothing else is given.
pub const DEFAULT_LIMIT: Duration = Duration::from_secs(15);

/// The stop key the CLI uses when nothing else is given.
pub const DEFAULT_KEY: &str = "esc";

/// How often the key is read.
const POLL: Duration = Duration::from_millis(30);

/// A key press this soon after the script stopped waiting belongs to the
/// wait (Esc closing a dialog), not to the running script.
const KEY_GRACE: Duration = Duration::from_millis(200);

/// Reads whether a virtual key is down (tests).
pub(crate) type KeyProbe = Arc<dyn Fn(u16) -> bool + Send + Sync>;

/// A key (with modifiers) that stops the script, like "esc" or
/// "ctrl+shift+q".
#[derive(Clone)]
pub struct StopKey {
    /// For messages ("Esc", "Ctrl+Shift+Q").
    pub name: String,
    /// The Windows virtual-key codes that must all be down.
    codes: Vec<u16>,
    pub(crate) probe: Option<KeyProbe>,
}

impl std::fmt::Debug for StopKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StopKey({} {:?})", self.name, self.codes)
    }
}

impl PartialEq for StopKey {
    fn eq(&self, o: &StopKey) -> bool {
        self.codes == o.codes
    }
}

const MODIFIERS: &[(&str, &str, u16)] = &[("ctrl", "Ctrl", 0x11), ("control", "Ctrl", 0x11), ("shift", "Shift", 0x10), ("alt", "Alt", 0x12)];

const KEYS: &[(&str, &str, u16)] = &[
    ("esc", "Esc", 0x1B),
    ("escape", "Esc", 0x1B),
    ("pause", "Pause", 0x13),
    ("break", "Pause", 0x13),
    ("space", "Space", 0x20),
    ("tab", "Tab", 0x09),
    ("enter", "Enter", 0x0D),
    ("backspace", "Backspace", 0x08),
    ("insert", "Insert", 0x2D),
    ("delete", "Delete", 0x2E),
    ("home", "Home", 0x24),
    ("end", "End", 0x23),
    ("pageup", "PageUp", 0x21),
    ("pagedown", "PageDown", 0x22),
    ("scrolllock", "ScrollLock", 0x91),
];

fn key_code(name: &str) -> Option<(String, u16)> {
    if let Some((_, n, c)) = KEYS.iter().find(|k| k.0 == name) {
        return Some((n.to_string(), *c));
    }
    let b = name.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
        let up = b[0].to_ascii_uppercase();
        return Some(((up as char).to_string(), up as u16));
    }
    let n: u16 = name.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then(|| (format!("F{n}"), 0x70 + n - 1))
}

impl StopKey {
    /// Parses "esc", "f12", "ctrl+q", "ctrl+shift+break"... (case and
    /// spaces do not matter). None when it is not a key.
    pub fn parse(s: &str) -> Option<StopKey> {
        let mut names = Vec::new();
        let mut codes = Vec::new();
        let mut key = None;
        for part in s.split('+') {
            let p = part.trim().to_lowercase();
            if let Some((_, n, c)) = MODIFIERS.iter().find(|m| m.0 == p) {
                if !codes.contains(c) {
                    names.push(n.to_string());
                    codes.push(*c);
                }
                continue;
            }
            if key.is_some() {
                return None;
            }
            key = Some(key_code(&p)?);
        }
        let (n, c) = key?;
        names.push(n);
        codes.push(c);
        Some(StopKey { name: names.join("+"), codes, probe: None })
    }

    fn down(&self) -> bool {
        self.codes.iter().all(|&c| match &self.probe {
            Some(p) => p(c),
            None => crate::ui::key_down(c),
        })
    }
}

/// What the waits, the key thread and the interrupt handler share. Times
/// are nanoseconds since base (pressed: 0 = no press).
struct State {
    base: Instant,
    active_since: AtomicU64,
    pressed: AtomicU64,
}

impl State {
    fn now(&self) -> u64 {
        self.base.elapsed().as_nanos() as u64 + 1
    }
}

thread_local! {
    /// The state of the script running on this thread.
    static CURRENT: RefCell<Option<Arc<State>>> = const { RefCell::new(None) };
}

/// Records that the current thread has just waited (the script is not
/// stuck, and a key pressed meanwhile was meant for the wait). Cheap; safe
/// on any thread.
pub(crate) fn waited() {
    CURRENT.with(|c| {
        if let Some(s) = &*c.borrow() {
            s.active_since.store(s.now(), Ordering::SeqCst);
            s.pressed.store(0, Ordering::SeqCst);
        }
    });
}

/// Ends the checks of install (drop it after removing the interrupt
/// handler).
pub(crate) struct Guard {
    quit: Arc<AtomicBool>,
    keys: Option<JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        if let Some(t) = self.keys.take() {
            let _ = t.join();
        }
        CURRENT.with(|c| c.borrow_mut().take());
    }
}

/// Sets up the watchdog (limit) and the stop key for a script run on this
/// thread in rt. None when both are off.
pub(crate) fn install(rt: &rquickjs::Runtime, host: Weak<Host>, limit: Option<Duration>, key: Option<StopKey>) -> Option<Guard> {
    if limit.is_none() && key.is_none() {
        return None;
    }
    let st = Arc::new(State { base: Instant::now(), active_since: AtomicU64::new(0), pressed: AtomicU64::new(0) });
    CURRENT.with(|c| *c.borrow_mut() = Some(st.clone()));
    waited();
    let quit = Arc::new(AtomicBool::new(false));
    let keys = key.clone().map(|k| {
        let (st, quit) = (st.clone(), quit.clone());
        std::thread::Builder::new()
            .name("stop-key".into())
            .spawn(move || {
                let mut was = false;
                while !quit.load(Ordering::SeqCst) {
                    let down = k.down();
                    if down && !was {
                        st.pressed.store(st.now(), Ordering::SeqCst);
                    }
                    was = down;
                    std::thread::sleep(POLL);
                }
            })
            .expect("cannot start the stop key thread")
    });
    let key_name = key.map(|k| k.name);
    let grace = KEY_GRACE.as_nanos() as u64;
    let mut stopped = false;
    rt.set_interrupt_handler(Some(Box::new(move || {
        if stopped {
            return true;
        }
        let active = st.active_since.load(Ordering::SeqCst);
        let pressed = st.pressed.swap(0, Ordering::SeqCst);
        let message = if pressed != 0 && pressed.saturating_sub(active) >= grace {
            t!("engine.stopKeyConfirm", key_name.as_deref().unwrap_or_default())
        } else if let Some(l) = limit
            && st.now().saturating_sub(active) >= l.as_nanos() as u64
        {
            t!("engine.stuckConfirm", l.as_secs_f64())
        } else {
            return false;
        };
        let Some(h) = host.upgrade() else { return false };
        let stop = h.env.ui.confirm(&h.env.title, &message);
        waited();
        if stop {
            stopped = true;
            let _ = writeln!(h.env.stderr.lock().unwrap(), "{}", t!("engine.stopped"));
            h.set_exit(STOPPED_CODE);
        }
        stop
    })));
    Some(Guard { quit, keys })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keys() {
        for (s, name, codes) in [
            ("esc", "Esc", vec![0x1B]),
            (" Escape ", "Esc", vec![0x1B]),
            ("F12", "F12", vec![0x7B]),
            ("ctrl+q", "Ctrl+Q", vec![0x11, 0x51]),
            ("Ctrl + Shift + Break", "Ctrl+Shift+Pause", vec![0x11, 0x10, 0x13]),
            ("alt+0", "Alt+0", vec![0x12, 0x30]),
        ] {
            let k = StopKey::parse(s).unwrap_or_else(|| panic!("{s:?}"));
            assert_eq!((k.name.as_str(), &k.codes), (name, &codes), "{s:?}");
        }
        for s in ["", "ctrl", "ctrl+", "q+w", "f25", "f0", "hoge", "ctrl+hoge"] {
            assert!(StopKey::parse(s).is_none(), "{s:?}");
        }
    }
}
