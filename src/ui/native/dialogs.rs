//! Dialogs built from in-memory templates: the prompt, the error dialog
//! (with selectable text and a Copy button) and the start screen's choices
//! (command links).

use std::cell::RefCell;

use unicode_width::UnicodeWidthStr;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::{hstr, owner, webview};
use crate::t;
use crate::ui::Choice;

const DS_SETFONT: u32 = 0x40;
const DS_MODALFRAME: u32 = 0x80;
const DS_SETFOREGROUND: u32 = 0x200;
const DS_CENTER: u32 = 0x800;
const SS_NOPREFIX: u32 = 0x80;
const SS_ICON: u32 = 0x3;
const ES_AUTOHSCROLL: u32 = 0x80;
const ES_MULTILINE: u32 = 0x4;
const ES_AUTOVSCROLL: u32 = 0x40;
const ES_READONLY: u32 = 0x800;
const BS_DEFPUSHBUTTON: u32 = 0x1;
const BS_COMMANDLINK: u32 = 0x0E;
const BS_DEFCOMMANDLINK: u32 = 0x0F;
const BCM_SETNOTE: u32 = 0x1609;
const BM_SETIMAGE: u32 = 0x00F7;
const STM_SETICON: u32 = 0x0170;
const EM_SETSEL: u32 = 0x00B1;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const CF_UNICODETEXT: u32 = 13;

const ID_OK: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_EDIT: i32 = 100;
const ID_COPY: i32 = 101;
const ID_TEXT: i32 = 102;
const ID_ICON: i32 = 103;
const ID_CHOICE: i32 = 1000; // + the index of the choice

/// (tests) Dialogs press their default button as soon as they open.
pub(crate) static AUTO_CLOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn auto_close(hwnd: HWND, ids: &[i32]) {
    if AUTO_CLOSE.load(std::sync::atomic::Ordering::SeqCst) {
        for id in ids {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(*id as usize), LPARAM(0));
            }
        }
    }
}

const CLASS_BUTTON: u16 = 0x0080;
const CLASS_EDIT: u16 = 0x0081;
const CLASS_STATIC: u16 = 0x0082;

/// Builds a DLGTEMPLATE in memory.
#[derive(Default)]
struct Builder {
    b: Vec<u8>,
}

impl Builder {
    fn u16(&mut self, v: u16) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        for c in s.encode_utf16() {
            self.u16(c);
        }
        self.u16(0);
    }
    fn align4(&mut self) {
        while !self.b.len().is_multiple_of(4) {
            self.b.push(0);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn item(&mut self, style: u32, x: i32, y: i32, cx: i32, cy: i32, id: i32, class: u16, text: &str) {
        self.align4();
        self.u32(style | WS_CHILD.0 | WS_VISIBLE.0);
        self.u32(0);
        for v in [x, y, cx, cy] {
            self.u16(v as i16 as u16);
        }
        self.u16(id as u16);
        self.u16(0xFFFF);
        self.u16(class);
        self.str(text);
        self.u16(0);
    }
    fn header(&mut self, ex_style: u32, count: usize, cx: i32, cy: i32, title: &str) {
        self.u32(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | DS_SETFONT | DS_MODALFRAME | DS_CENTER | DS_SETFOREGROUND);
        self.u32(ex_style);
        self.u16(count as u16);
        self.u16(0);
        self.u16(0);
        self.u16(cx as u16);
        self.u16(cy as u16);
        self.u16(0); // menu
        self.u16(0); // class
        self.str(title);
        self.u16(9);
        self.str("Segoe UI"); // CJK text falls back through font linking
    }
    /// The template as DWORD-aligned memory.
    fn words(&self) -> Vec<u32> {
        let mut w = vec![0u32; self.b.len().div_ceil(4)];
        for (i, c) in self.b.chunks(4).enumerate() {
            let mut a = [0u8; 4];
            a[..c.len()].copy_from_slice(c);
            w[i] = u32::from_le_bytes(a);
        }
        w
    }
}

fn run_dialog(words: &[u32], parent: Option<HWND>, proc_: DLGPROC) -> isize {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        DialogBoxIndirectParamW(Some(hinst.into()), words.as_ptr() as *const DLGTEMPLATE, parent, proc_, LPARAM(0))
    }
}

fn item(hwnd: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(hwnd), id).unwrap_or_default() }
}

fn set_text(hwnd: HWND, id: i32, s: &str) {
    unsafe {
        let _ = SetDlgItemTextW(hwnd, id, &hstr(s));
    }
}

/// The number of lines text takes in width dialog units (about 4 DLU per
/// half-width character).
fn text_lines(s: &str, width: i32) -> i32 {
    s.replace("\r\n", "\n").split('\n').map(|p| (((p.width() as i32) * 4 + width - 1) / width).max(1)).sum()
}

// ---- prompt ----

struct PromptState {
    def: String,
    result: Option<String>,
}

thread_local! {
    static PROMPT: RefCell<Option<PromptState>> = const { RefCell::new(None) };
    static ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    static CHOOSE: RefCell<Option<ChooseState>> = const { RefCell::new(None) };
}

unsafe extern "system" fn prompt_proc(hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                let _ = SetForegroundWindow(hwnd);
                let def = PROMPT.with(|p| p.borrow().as_ref().map(|s| s.def.clone()).unwrap_or_default());
                set_text(hwnd, ID_EDIT, &def);
                let edit = item(hwnd, ID_EDIT);
                SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                let _ = SetFocus(Some(edit));
                auto_close(hwnd, &[ID_OK]);
                0
            }
            WM_COMMAND => match (wparam.0 & 0xFFFF) as i32 {
                ID_OK => {
                    let edit = item(hwnd, ID_EDIT);
                    let n = GetWindowTextLengthW(edit);
                    let mut buf = vec![0u16; n as usize + 1];
                    let got = GetDlgItemTextW(hwnd, ID_EDIT, &mut buf);
                    let s = String::from_utf16_lossy(&buf[..got as usize]);
                    PROMPT.with(|p| {
                        if let Some(st) = p.borrow_mut().as_mut() {
                            st.result = Some(s);
                        }
                    });
                    let _ = EndDialog(hwnd, ID_OK as isize);
                    1
                }
                ID_CANCEL => {
                    let _ = EndDialog(hwnd, ID_CANCEL as isize);
                    1
                }
                _ => 0,
            },
            WM_CLOSE => {
                let _ = EndDialog(hwnd, ID_CANCEL as isize);
                1
            }
            _ => 0,
        }
    }
}

/// Asks for a line of text; None when cancelled.
pub(super) fn prompt(title: &str, message: &str, def: &str) -> Option<String> {
    const CX: i32 = 240;
    const MARGIN: i32 = 7;
    let text_w = CX - 2 * MARGIN;
    let text_h = text_lines(message, text_w).min(20) * 9;
    let edit_y = MARGIN + text_h + 4;
    let btn_y = edit_y + 14 + 7;
    let cy = btn_y + 14 + MARGIN;
    let mut d = Builder::default();
    d.header(0, 4, CX, cy, title);
    d.item(SS_NOPREFIX, MARGIN, MARGIN, text_w, text_h, 0xFFFF, CLASS_STATIC, message);
    d.item(WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL, MARGIN, edit_y, text_w, 14, ID_EDIT, CLASS_EDIT, "");
    d.item(WS_TABSTOP.0 | BS_DEFPUSHBUTTON, CX - MARGIN - 50 - 4 - 50, btn_y, 50, 14, ID_OK, CLASS_BUTTON, "OK");
    d.item(WS_TABSTOP.0, CX - MARGIN - 50, btn_y, 50, 14, ID_CANCEL, CLASS_BUTTON, &t!("ui.cancel"));
    PROMPT.with(|p| *p.borrow_mut() = Some(PromptState { def: def.to_string(), result: None }));
    run_dialog(&d.words(), owner(), Some(prompt_proc));
    PROMPT.with(|p| p.borrow_mut().take()).and_then(|s| s.result)
}

// ---- error dialog ----

fn set_clipboard_text(hwnd: HWND, s: &str) -> bool {
    let mut u: Vec<u16> = s.encode_utf16().collect();
    u.push(0);
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let size = u.len() * 2;
        let ok = (|| {
            let h = GlobalAlloc(GMEM_MOVEABLE, size).ok()?;
            let p = GlobalLock(h);
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return None;
            }
            std::ptr::copy_nonoverlapping(u.as_ptr() as *const u8, p as *mut u8, size);
            let _ = GlobalUnlock(h);
            if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(h.0))).is_err() {
                let _ = GlobalFree(Some(HGLOBAL(h.0)));
                return None;
            }
            Some(()) // the clipboard owns the memory now
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

unsafe extern "system" fn error_proc(hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                let _ = SetForegroundWindow(hwnd);
                if let Ok(icon) = LoadIconW(None, IDI_ERROR) {
                    SendMessageW(item(hwnd, ID_ICON), STM_SETICON, Some(WPARAM(icon.0 as usize)), Some(LPARAM(0)));
                }
                SendMessageW(item(hwnd, ID_TEXT), EM_SETLIMITTEXT, Some(WPARAM(0)), Some(LPARAM(0)));
                let text = ERROR.with(|e| e.borrow().clone().unwrap_or_default());
                set_text(hwnd, ID_TEXT, &text);
                let _ = SetFocus(Some(item(hwnd, ID_OK)));
                auto_close(hwnd, &[ID_COPY, ID_OK]);
                0
            }
            WM_COMMAND => match (wparam.0 & 0xFFFF) as i32 {
                ID_COPY => {
                    let text = ERROR.with(|e| e.borrow().clone().unwrap_or_default());
                    if set_clipboard_text(hwnd, &text) {
                        set_text(hwnd, ID_COPY, &t!("ui.copied"));
                    }
                    1
                }
                ID_OK | ID_CANCEL => {
                    let _ = EndDialog(hwnd, ID_OK as isize);
                    1
                }
                _ => 0,
            },
            WM_CLOSE => {
                let _ = EndDialog(hwnd, ID_OK as isize);
                1
            }
            _ => 0,
        }
    }
}

/// Shows an error whose text can be selected and copied. It returns false
/// when the dialog could not be shown.
pub(super) fn error_dialog(title: &str, message: &str) -> bool {
    let text = message.replace("\r\n", "\n").replace('\n', "\r\n").replace('\t', "    ");
    const CX: i32 = 340;
    const MARGIN: i32 = 7;
    const ICON_W: i32 = 21;
    const TEXT_X: i32 = MARGIN + ICON_W + 6;
    const TEXT_W: i32 = CX - TEXT_X - MARGIN;
    const BTN_H: i32 = 14;
    let lines: i32 = text.split("\r\n").map(|l| (((l.width() as i32) * 4 + TEXT_W - 12) / (TEXT_W - 12)).max(1)).sum();
    let text_h = (lines * 9 + 6).clamp(34, 190);
    let btn_y = MARGIN + text_h + 7;
    let cy = btn_y + BTN_H + MARGIN;
    let copy_w = 60;
    let mut d = Builder::default();
    d.header(WS_EX_TOPMOST.0, 4, CX, cy, title);
    d.item(SS_ICON, MARGIN, MARGIN, ICON_W, 20, ID_ICON, CLASS_STATIC, "");
    d.item(
        WS_BORDER.0 | WS_VSCROLL.0 | WS_TABSTOP.0 | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL,
        TEXT_X,
        MARGIN,
        TEXT_W,
        text_h,
        ID_TEXT,
        CLASS_EDIT,
        "",
    );
    d.item(WS_TABSTOP.0, CX - MARGIN - 50 - 4 - copy_w, btn_y, copy_w, BTN_H, ID_COPY, CLASS_BUTTON, &t!("ui.copy"));
    d.item(WS_TABSTOP.0 | BS_DEFPUSHBUTTON, CX - MARGIN - 50, btn_y, 50, BTN_H, ID_OK, CLASS_BUTTON, "OK");
    ERROR.with(|e| *e.borrow_mut() = Some(text));
    unsafe {
        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONERROR);
    }
    let r = run_dialog(&d.words(), None, Some(error_proc));
    ERROR.with(|e| e.borrow_mut().take());
    r != 0 && r != -1
}

// ---- choices ----

struct ChooseState {
    choices: Vec<Choice>,
    result: i64,
    icons: Vec<HICON>,
}

const CHOICE_ICON_SIZE: u32 = 32; // logical pixels

unsafe extern "system" fn choose_proc(hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                let _ = SetForegroundWindow(hwnd);
                let icons = webview::exe_icons();
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(icons.0.0 as isize)));
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(icons.1.0 as isize)));
                let mut dpi = GetDpiForSystem();
                if dpi == 0 {
                    dpi = 96;
                }
                CHOOSE.with(|c| {
                    if let Some(st) = c.borrow_mut().as_mut() {
                        for (i, ch) in st.choices.clone().iter().enumerate() {
                            let btn = item(hwnd, ID_CHOICE + i as i32);
                            if !ch.note.is_empty() {
                                let note = hstr(&ch.note);
                                SendMessageW(btn, BCM_SETNOTE, Some(WPARAM(0)), Some(LPARAM(note.as_ptr() as isize)));
                            }
                            if !ch.icon.is_empty()
                                && let Some(icon) = webview::icon_from_bytes(&ch.icon, CHOICE_ICON_SIZE * dpi / 96)
                            {
                                st.icons.push(icon);
                                SendMessageW(btn, BM_SETIMAGE, Some(WPARAM(IMAGE_ICON.0 as usize)), Some(LPARAM(icon.0 as isize)));
                            }
                        }
                    }
                });
                let _ = SetFocus(Some(item(hwnd, ID_CHOICE)));
                auto_close(hwnd, &[ID_CHOICE]);
                0
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let n = CHOOSE.with(|c| c.borrow().as_ref().map(|s| s.choices.len()).unwrap_or(0)) as i32;
                if (ID_CHOICE..ID_CHOICE + n).contains(&id) {
                    CHOOSE.with(|c| {
                        if let Some(st) = c.borrow_mut().as_mut() {
                            st.result = (id - ID_CHOICE) as i64;
                        }
                    });
                    let _ = EndDialog(hwnd, ID_OK as isize);
                    return 1;
                }
                if id == ID_CANCEL {
                    let _ = EndDialog(hwnd, ID_CANCEL as isize);
                    return 1;
                }
                0
            }
            WM_CLOSE => {
                let _ = EndDialog(hwnd, ID_CANCEL as isize);
                1
            }
            _ => 0,
        }
    }
}

/// Asks which of a few actions to take, shown as command links; the index
/// of the choice or -1 when cancelled.
pub(super) fn choose(title: &str, message: &str, choices: &[Choice]) -> i64 {
    const CX: i32 = 280;
    const MARGIN: i32 = 7;
    const TEXT_W: i32 = CX - 2 * MARGIN;
    const BTN_H: i32 = 14;
    let mut y = MARGIN;
    let mut items = Builder::default();
    let mut count = 0;
    if !message.is_empty() {
        let h = text_lines(message, TEXT_W) * 9;
        items.item(SS_NOPREFIX, MARGIN, y, TEXT_W, h, 0xFFFF, CLASS_STATIC, message);
        count += 1;
        y += h + 4;
    }
    for (i, c) in choices.iter().enumerate() {
        let mut h = 18;
        if !c.note.is_empty() {
            h += text_lines(&c.note, TEXT_W - 40) * 9;
        }
        if !c.icon.is_empty() && h < 26 {
            h = 26;
        }
        let style = WS_TABSTOP.0 | if i == 0 { BS_DEFCOMMANDLINK } else { BS_COMMANDLINK };
        items.item(style, MARGIN, y, TEXT_W, h, ID_CHOICE + i as i32, CLASS_BUTTON, &c.label);
        count += 1;
        y += h + 2;
    }
    y += 5;
    items.item(WS_TABSTOP.0, CX - MARGIN - 50, y, 50, BTN_H, ID_CANCEL, CLASS_BUTTON, &t!("ui.cancel"));
    count += 1;
    let cy = y + BTN_H + MARGIN;
    let mut d = Builder::default();
    d.header(0, count, CX, cy, title);
    d.align4();
    d.b.extend_from_slice(&items.b); // items.b starts DWORD aligned
    CHOOSE.with(|c| *c.borrow_mut() = Some(ChooseState { choices: choices.to_vec(), result: -1, icons: Vec::new() }));
    run_dialog(&d.words(), owner(), Some(choose_proc));
    let st = CHOOSE.with(|c| c.borrow_mut().take());
    match st {
        Some(st) => {
            for icon in st.icons {
                unsafe {
                    let _ = DestroyIcon(icon);
                }
            }
            st.result
        }
        None => -1,
    }
}
