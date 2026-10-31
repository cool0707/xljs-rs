//! The UI thread of the native UI: the main thread running the Win32
//! message loop. Queued functions run from a message-only window, so they
//! also run while a modal dialog has its own loop.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::ui::thread::{ChanThread, Job, UiThread};

const WM_RUN_QUEUED: u32 = WM_APP + 1;

static QUEUE: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
static CLASS: OnceLock<bool> = OnceLock::new();

fn drain() {
    loop {
        let job = QUEUE.lock().unwrap().pop_front();
        match job {
            Some(j) => j(),
            None => return,
        }
    }
}

unsafe extern "system" fn dispatch_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_RUN_QUEUED {
        drain();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

struct WinThread {
    hwnd: isize,
}

pub(crate) fn new_ui_thread() -> Arc<dyn UiThread> {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        CLASS.get_or_init(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(dispatch_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("xljsDispatch"),
                ..Default::default()
            };
            RegisterClassExW(&wc) != 0
        });
        match CreateWindowExW(Default::default(), w!("xljsDispatch"), None, Default::default(), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst.into()), None) {
            Ok(hwnd) => Arc::new(WinThread { hwnd: hwnd.0 as isize }),
            Err(_) => Arc::new(ChanThread::default()),
        }
    }
}

impl UiThread for WinThread {
    fn post(&self, f: Job) {
        QUEUE.lock().unwrap().push_back(f);
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_RUN_QUEUED, WPARAM(0), LPARAM(0));
        }
    }

    fn serve(&self, done: &Arc<(Mutex<bool>, Condvar)>) {
        let mut m = MSG::default();
        loop {
            if *done.0.lock().unwrap() {
                drain();
                unsafe {
                    let _ = DestroyWindow(HWND(self.hwnd as *mut _));
                }
                return;
            }
            let r = unsafe { GetMessageW(&mut m, None, 0, 0) };
            if r.0 <= 0 {
                // A stray WM_QUIT (or an error): keep serving until the
                // script ends.
                continue;
            }
            unsafe {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }

    // Calls into the progress dialog are cross-thread round trips; scripts
    // check for cancellation once per item, so reuse the answer briefly.
    fn cancel_poll(&self) -> Duration {
        Duration::from_millis(50)
    }
}
