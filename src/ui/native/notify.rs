//! Notifications at the bottom right of the screen: a notification area
//! icon (Shell_NotifyIcon) and its balloon, which Windows 10 and 11 show as
//! a toast. This needs no AppUserModelID or Start menu shortcut, so it works
//! from a lone exe.
//!
//! The icon is added for the first notification and removed once every
//! balloon has been closed or has timed out (or after REMOVE_AFTER at the
//! latest, for balloons Windows does not report on). Clicking a
//! notification brings the topmost window of xljs (a dialog, say) to the
//! front.
//!
//! The same notifications are shown automatically when a dialog or window
//! opens while the user works in another application (watch): Windows does
//! not let a background process take the foreground, so the dialog would
//! otherwise only flash on the taskbar.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::GetConsoleWindow;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, w};

use super::webview::exe_icons;

const WM_TRAY: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;
const REMOVE_TIMER: usize = 1;
/// How long the icon may stay without news from Windows.
const REMOVE_AFTER: Duration = Duration::from_secs(30);
/// How long the process waits at exit for a notification still shown
/// (removing the icon would take it away).
const EXIT_WAIT: Duration = Duration::from_secs(10);
/// How long a dialog has to come to the front before a notification is
/// shown for it.
const WATCH_DELAY: u32 = 500;

// The notification events (NOTIFYICON_VERSION_4: the low word of lParam).
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
const NIN_BALLOONHIDE: u32 = WM_USER + 3;
const NIN_BALLOONTIMEOUT: u32 = WM_USER + 4;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

static CLASS: OnceLock<bool> = OnceLock::new();

#[derive(Default)]
struct State {
    hwnd: Option<HWND>,
    added: bool,
    /// Balloons shown and not yet reported closed.
    open: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The notifications to show if the dialog has not come to the front
    /// by the time the timer fires, by timer id.
    static WATCHES: RefCell<HashMap<usize, (String, String)>> = RefCell::new(HashMap::new());
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match (lparam.0 as u32) & 0xFFFF {
                NIN_BALLOONUSERCLICK => {
                    activate();
                    closed();
                }
                NIN_BALLOONHIDE | NIN_BALLOONTIMEOUT => closed(),
                NIN_SELECT | NIN_KEYSELECT => activate(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == REMOVE_TIMER => {
            remove();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The hidden window that owns the icon, created on first use.
fn window() -> Option<HWND> {
    if let Some(h) = STATE.with(|s| s.borrow().hwnd) {
        return Some(h);
    }
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        CLASS.get_or_init(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinst.into(),
                lpszClassName: w!("xljsNotify"),
                ..Default::default()
            };
            RegisterClassExW(&wc) != 0
        });
        // A hidden top-level window: message-only windows do not get every
        // notification area message.
        let h = CreateWindowExW(WS_EX_TOOLWINDOW, w!("xljsNotify"), w!("xljs"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst.into()), None).ok()?;
        STATE.with(|s| s.borrow_mut().hwnd = Some(h));
        Some(h)
    }
}

/// Copies s into a fixed-size UTF-16 buffer with its terminating NUL,
/// shortening it with "…" when it does not fit.
fn copy_to(dst: &mut [u16], s: &str) {
    let s = s.split('\0').next().unwrap_or("");
    let mut v: Vec<u16> = s.encode_utf16().collect();
    if v.len() >= dst.len() {
        v.truncate(dst.len() - 2);
        // Do not leave half of a surrogate pair.
        if v.last().is_some_and(|c| (0xD800..0xDC00).contains(c)) {
            v.pop();
        }
        v.push('…' as u16);
    }
    dst[..v.len()].copy_from_slice(&v);
    dst[v.len()] = 0;
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() }
}

/// Shows a notification. Must be called on the UI thread.
pub(crate) fn show(title: &str, message: &str) {
    let Some(hwnd) = window() else { return };
    // A balloon without text is not shown: use the title as the text.
    let (title, message) = if message.trim().is_empty() { ("", title) } else { (title, message) };
    let message = if message.trim().is_empty() { " " } else { message };
    let (big, small) = exe_icons();
    let mut d = base_data(hwnd);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_INFO | NIF_SHOWTIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = small;
    copy_to(&mut d.szTip, if title.is_empty() { "xljs" } else { title });
    copy_to(&mut d.szInfoTitle, title);
    copy_to(&mut d.szInfo, message);
    if big.is_invalid() {
        d.dwInfoFlags = NIIF_INFO;
    } else {
        d.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        d.hBalloonIcon = big;
    }
    unsafe {
        let added = STATE.with(|s| s.borrow().added);
        if added {
            if !Shell_NotifyIconW(NIM_MODIFY, &d).as_bool() {
                return;
            }
        } else {
            if !Shell_NotifyIconW(NIM_ADD, &d).as_bool() {
                return;
            }
            let mut v = base_data(hwnd);
            v.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &v);
        }
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.added = true;
            s.open += 1;
        });
        SetTimer(Some(hwnd), REMOVE_TIMER, REMOVE_AFTER.as_millis() as u32, None);
    }
}

/// A balloon was closed: the icon goes with the last one.
fn closed() {
    let last = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.open = s.open.saturating_sub(1);
        s.open == 0
    });
    if last {
        remove();
    }
}

/// Removes the icon (and with it any notification still shown).
fn remove() {
    let (hwnd, added) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let r = (s.hwnd, s.added);
        s.added = false;
        s.open = 0;
        r
    });
    if let Some(h) = hwnd {
        unsafe {
            let _ = KillTimer(Some(h), REMOVE_TIMER);
            if added {
                let _ = Shell_NotifyIconW(NIM_DELETE, &base_data(h));
            }
        }
    }
}

/// Waits (serving window messages) until the notifications still shown
/// have closed, at most EXIT_WAIT, then removes the icon. Called when the
/// script has ended, before the process exits.
pub(crate) fn finish() {
    let deadline = Instant::now() + EXIT_WAIT;
    let mut m = MSG::default();
    while STATE.with(|s| s.borrow().added) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        unsafe {
            MsgWaitForMultipleObjects(None, false, (deadline - now).as_millis() as u32, QS_ALLINPUT);
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }
    remove();
    if let Some(h) = STATE.with(|s| s.borrow_mut().hwnd.take()) {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

/// Whether the user works in xljs: the foreground window is one of this
/// process, or the console xljs runs in.
pub(crate) fn app_active() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return true;
        }
        let con = GetConsoleWindow();
        if con.is_invalid() {
            return false;
        }
        // Windows Terminal: the console window is hidden and owned by the
        // terminal's window.
        con == fg || GetAncestor(con, GA_ROOTOWNER) == fg || GetWindow(con, GW_OWNER).is_ok_and(|o| o == fg)
    }
}

unsafe extern "system" fn watch_proc(_: HWND, _: u32, id: usize, _: u32) {
    unsafe {
        let _ = KillTimer(None, id);
    }
    if let Some((title, message)) = WATCHES.with(|w| w.borrow_mut().remove(&id))
        && super::super::auto_notify()
        && !app_active()
    {
        show(&title, &message);
    }
}

/// Shows a notification shortly unless xljs is in the foreground by then
/// (a dialog that is about to open came to the front). The timer also runs
/// inside a dialog's modal loop. Returns an id for unwatch (0 = none).
pub(crate) fn watch(title: &str, message: &str) -> usize {
    if !super::super::auto_notify() {
        return 0;
    }
    let id = unsafe { SetTimer(None, 0, WATCH_DELAY, Some(watch_proc)) };
    if id != 0 {
        WATCHES.with(|w| w.borrow_mut().insert(id, (title.to_string(), message.to_string())));
    }
    id
}

/// Cancels a watch whose dialog has closed.
pub(crate) fn unwatch(id: usize) {
    if id == 0 {
        return;
    }
    unsafe {
        let _ = KillTimer(None, id);
    }
    WATCHES.with(|w| w.borrow_mut().remove(&id));
}

/// Runs f (which shows a modal dialog) with a watch.
pub(crate) fn watching<T>(title: &str, message: &str, f: impl FnOnce() -> T) -> T {
    let id = watch(title, message);
    let r = f();
    unwatch(id);
    r
}

unsafe extern "system" fn find_top(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() && IsWindowEnabled(hwnd).as_bool() {
            *(lparam.0 as *mut Option<HWND>) = Some(hwnd);
            return false.into();
        }
        true.into()
    }
}

/// Brings the topmost window of this process to the front (the user
/// clicked a notification, which lets the process take the foreground).
fn activate() {
    let mut found: Option<HWND> = None;
    unsafe {
        // EnumWindows goes from the top of the Z order down.
        let _ = EnumWindows(Some(find_top), LPARAM(&mut found as *mut _ as isize));
        if let Some(h) = found {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::copy_to;

    #[test]
    fn copy_to_truncates() {
        let mut b = [1u16; 8];
        copy_to(&mut b, "abc");
        assert_eq!(&b[..4], &[97, 98, 99, 0]);
        copy_to(&mut b, "abcdefghijk");
        assert_eq!(String::from_utf16_lossy(&b[..7]), "abcdef…");
        assert_eq!(b[7], 0);
        // A surrogate pair is not split.
        copy_to(&mut b, "abcde😀xyz");
        assert_eq!(String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap()]), "abcde…");
    }
}
