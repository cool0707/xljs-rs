use std::io::{BufRead, IsTerminal};

use crate::ui::Ui;
use crate::{Error, i18n, t};

/// How the process is attached to a console.
pub struct ConsoleState {
    /// stdout/stderr reach a console or a redirect.
    pub has: bool,
    /// The console window was created for this process (e.g. the exe was
    /// double-clicked) and closes when it exits.
    pub own: bool,
}

impl ConsoleState {
    /// Whether the process was not started from a terminal: it got a console
    /// of its own (xljsc.exe double-clicked) or none at all (xljs.exe).
    pub fn from_explorer(&self) -> bool {
        self.own || !self.has
    }

    pub fn finish(&self) {
        if self.own && std::io::stdin().is_terminal() {
            eprint!("\n{}", t!("cli.pressEnterToClose"));
            let mut s = String::new();
            let _ = std::io::stdin().lock().read_line(&mut s);
        }
    }

    /// Reports err and returns the exit code 1. Without a console the error
    /// is shown in a dialog so that GUI builds never fail silently.
    pub fn fail(&self, u: Option<&dyn Ui>, e: &Error) -> i32 {
        let msg = match e {
            Error::Bundle(_) => format!("{}\n{e}", t!("cli.syntaxError")),
            Error::Script(_) => e.to_string(),
            _ => t!("cli.error", i18n::error(e)),
        };
        eprintln!("{msg}");
        if !self.has {
            let title = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "xljs".into());
            match u {
                Some(u) if u.name() == "native" => u.show_error(&title, &msg),
                None if crate::ui::native_available() => {
                    if let Ok(n) = crate::ui::new_native() {
                        n.show_error(&title, &msg);
                    }
                }
                _ => {}
            }
        }
        1
    }
}

#[cfg(not(windows))]
pub fn setup() -> ConsoleState {
    ConsoleState { has: true, own: false }
}

/// Works out whether output can be seen. A GUI-subsystem build (xljs.exe)
/// started from a terminal attaches to that terminal.
#[cfg(windows)]
pub fn setup() -> ConsoleState {
    use windows::Win32::System::Console::*;
    unsafe {
        if !GetConsoleWindow().is_invalid() {
            let mut pids = [0u32; 4];
            let n = GetConsoleProcessList(&mut pids);
            return ConsoleState { has: true, own: n == 1 };
        }
        let valid = |h: windows::core::Result<windows::Win32::Foundation::HANDLE>| h.map(|h| !h.is_invalid() && !h.0.is_null()).unwrap_or(false);
        let out_ok = valid(GetStdHandle(STD_OUTPUT_HANDLE));
        let err_ok = valid(GetStdHandle(STD_ERROR_HANDLE));
        if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            // Rust's std reopens the standard handles on each write, so
            // setting them is enough.
            let open = || {
                std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$").ok()
            };
            use std::os::windows::io::IntoRawHandle;
            if !out_ok && let Some(f) = open() {
                let _ = SetStdHandle(STD_OUTPUT_HANDLE, windows::Win32::Foundation::HANDLE(f.into_raw_handle()));
            }
            if !err_ok && let Some(f) = open() {
                let _ = SetStdHandle(STD_ERROR_HANDLE, windows::Win32::Foundation::HANDLE(f.into_raw_handle()));
            }
            return ConsoleState { has: true, own: false };
        }
        ConsoleState { has: out_ok || err_ok, own: false }
    }
}
