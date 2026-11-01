//! Runs the script being developed in a process of its own (this exe), so
//! that a crash, exit() or an endless loop cannot take the development
//! window with it. Its output and its end come back as events.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{Error, Result};

/// What a run reports to the session.
#[derive(Debug)]
pub enum RunEvent {
    Output { id: u64, stderr: bool, text: String },
    Exited { id: u64, code: Option<i32> },
}

/// A running script.
pub struct Run {
    pub id: u64,
    child: Arc<Mutex<Option<Child>>>,
}

/// The environment variable that tells a run it was started by the
/// development mode (fs.remove asks before deleting much).
pub const DEV_ENV: &str = "XLJS_DEV";
/// The environment variable with the file for the form preview.
pub const PREVIEW_ENV: &str = "XLJS_DEV_PREVIEW";

impl Run {
    /// Starts exe with args in dir; events go to tx (wrapped by wrap).
    pub fn start<E: Send + 'static>(
        exe: &Path,
        args: &[String],
        dir: &Path,
        env: &[(String, String)],
        id: u64,
        tx: Sender<E>,
        wrap: fn(RunEvent) -> E,
    ) -> Result<Run> {
        let mut cmd = Command::new(exe);
        cmd.args(args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| Error::path("exec", exe.display().to_string(), e))?;
        let mut readers = Vec::new();
        let out: Option<Box<dyn Read + Send>> = child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        let err: Option<Box<dyn Read + Send>> = child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>);
        for (stream, is_err) in [(out, false), (err, true)] {
            let Some(mut stream) = stream else { continue };
            let tx = tx.clone();
            readers.push(std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                let mut pending: Vec<u8> = Vec::new();
                loop {
                    let n = match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    pending.extend_from_slice(&buf[..n]);
                    // Whole UTF-8 characters only.
                    let cut = match std::str::from_utf8(&pending) {
                        Ok(_) => pending.len(),
                        Err(e) if e.error_len().is_none() => e.valid_up_to(),
                        Err(_) => pending.len(),
                    };
                    if cut == 0 {
                        continue;
                    }
                    let text = super::project::decode(&pending[..cut]);
                    pending.drain(..cut);
                    let _ = tx.send(wrap(RunEvent::Output { id, stderr: is_err, text }));
                }
                if !pending.is_empty() {
                    let _ = tx.send(wrap(RunEvent::Output { id, stderr: is_err, text: super::project::decode(&pending) }));
                }
            }));
        }
        let child = Arc::new(Mutex::new(Some(child)));
        let c2 = child.clone();
        std::thread::spawn(move || {
            let code = loop {
                {
                    let mut g = c2.lock().unwrap();
                    match g.as_mut().map(|c| c.try_wait()) {
                        Some(Ok(Some(st))) => break st.code(),
                        Some(Ok(None)) => {}
                        _ => break None,
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            for r in readers {
                let _ = r.join();
            }
            c2.lock().unwrap().take();
            let _ = tx.send(wrap(RunEvent::Exited { id, code }));
        });
        Ok(Run { id, child })
    }

    /// Ends the process (the Exited event follows).
    pub fn kill(&self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }

    pub fn running(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }
}
