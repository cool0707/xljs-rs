//! The native UI on Windows: Win32 dialogs, IFileDialog, IProgressDialog
//! and HTML windows (WebView2). Everything here runs on the UI thread (the
//! main thread, see ui::run); the types are Send + Sync only so that the
//! UI can be handed around, and must not be used from other threads.

mod browser;
mod dialogs;
mod loader;
mod notify;
mod thread;
mod webview;

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use super::*;
use crate::{Error, Result, err};

pub(crate) use notify::finish as finish_notifications;
pub(crate) use thread::new_ui_thread;
pub(crate) use webview::webview2_version;

/// Implements Ui with Win32 dialogs. All methods must be called from the
/// thread that created it.
pub struct Native;

thread_local! {
    /// The open HTML windows, oldest first.
    static WINDOWS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

/// The window dialogs belong to: the newest open HTML window.
pub(crate) fn owner() -> Option<HWND> {
    WINDOWS.with(|w| w.borrow().last().copied())
}

thread_local! {
    /// The result of initialising COM on this thread (COM is per thread:
    /// tests, for one, run each on a thread of its own).
    static COM: std::result::Result<(), String> = unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        // S_FALSE (already initialised) and RPC_E_CHANGED_MODE are fine.
        if hr.is_err() && hr.0 as u32 != 0x80010106 {
            Err(format!("CoInitializeEx failed: HRESULT 0x{:08X}", hr.0 as u32))
        } else {
            Ok(())
        }
    };
}

impl Native {
    /// Initialises COM on the calling thread and returns the native UI.
    pub fn new() -> Result<Arc<dyn Ui>> {
        match COM.with(|r| r.clone()) {
            Ok(()) => Ok(Arc::new(Native)),
            Err(e) => Err(Error::msg(e)),
        }
    }
}

/// Whether new_native can succeed.
pub(crate) fn available() -> bool {
    true
}

/// Whether the virtual key vk is down now (any thread, any focus).
pub(crate) fn key_down(vk: u16) -> bool {
    let state = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk as i32) };
    (state as u16) & 0x8000 != 0
}

pub(crate) fn hstr(s: &str) -> HSTRING {
    // A NUL ends the text.
    HSTRING::from(s.split('\0').next().unwrap_or(""))
}

fn message_box(title: &str, message: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    notify::watching(title, message, || unsafe { MessageBoxW(owner(), &hstr(message), &hstr(title), flags | MB_SETFOREGROUND | MB_TOPMOST) })
}

/// The title of the notification shown for a picker that opens in the
/// background.
fn waiting_title(title: &str) -> String {
    if title.is_empty() { "xljs".into() } else { title.to_string() }
}

const ERROR_CANCELLED_HR: u32 = 0x800704C7;

fn shell_item_path(item: &IShellItem) -> Result<String> {
    unsafe {
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| Error::msg(format!("IShellItem.GetDisplayName: {e}")))?;
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(s)
    }
}

fn file_dialog(save: bool, folder: bool, o: &PickerOptions) -> Result<Vec<String>> {
    unsafe {
        let dlg: IFileDialog = if save {
            CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| Error::msg(format!("CoCreateInstance: {e}")))?
                .into()
        } else {
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| Error::msg(format!("CoCreateInstance: {e}")))?
                .into()
        };
        let mut opts = dlg.GetOptions().unwrap_or_default();
        opts |= FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR;
        if folder {
            opts |= FOS_PICKFOLDERS | FOS_PATHMUSTEXIST;
        } else if save {
            opts |= FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST;
        } else {
            opts |= FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST;
            if o.multiple {
                opts |= FOS_ALLOWMULTISELECT;
            }
        }
        let _ = dlg.SetOptions(opts);
        if !o.title.is_empty() {
            let _ = dlg.SetTitle(&hstr(&o.title));
        }
        // The strings must outlive the calls.
        let mut keep: Vec<HSTRING> = Vec::new();
        if !folder && !o.filters.is_empty() {
            let mut specs = Vec::new();
            for f in &o.filters {
                let pat = f.patterns.join(";");
                let name = if f.description.is_empty() { pat.clone() } else { f.description.clone() };
                keep.push(hstr(&name));
                keep.push(hstr(&pat));
            }
            for i in 0..o.filters.len() {
                specs.push(windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC { pszName: PCWSTR(keep[2 * i].as_ptr()), pszSpec: PCWSTR(keep[2 * i + 1].as_ptr()) });
            }
            let _ = dlg.SetFileTypes(&specs);
            if save
                && let Some(first) = o.filters[0].patterns.first()
            {
                let ext = first.trim_start_matches("*.");
                if !ext.is_empty() && !ext.contains(['*', '?']) {
                    let _ = dlg.SetDefaultExtension(&hstr(ext));
                }
            }
        }
        if !o.start_in.is_empty()
            && let Ok(abs) = std::path::absolute(&o.start_in)
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&hstr(&abs.to_string_lossy()), None)
        {
            let _ = dlg.SetFolder(&item);
        }
        if save && !o.suggested_name.is_empty() {
            let _ = dlg.SetFileName(&hstr(&o.suggested_name));
        }
        if let Err(e) = notify::watching(&waiting_title(&o.title), &crate::t!("ui.waiting"), || dlg.Show(owner())) {
            if e.code().0 as u32 == ERROR_CANCELLED_HR {
                return Ok(Vec::new());
            }
            return Err(Error::msg(format!("IFileDialog.Show: {e}")));
        }
        if !save && !folder && o.multiple {
            let open: IFileOpenDialog = windows::core::Interface::cast(&dlg).map_err(|e| Error::msg(e.to_string()))?;
            let arr = open.GetResults().map_err(|e| Error::msg(format!("GetResults: {e}")))?;
            let n = arr.GetCount().unwrap_or(0);
            let mut out = Vec::new();
            for i in 0..n {
                if let Ok(item) = arr.GetItemAt(i)
                    && let Ok(p) = shell_item_path(&item)
                {
                    out.push(p);
                }
            }
            return Ok(out);
        }
        let item = dlg.GetResult().map_err(|e| Error::msg(format!("GetResult: {e}")))?;
        Ok(vec![shell_item_path(&item)?])
    }
}

impl Ui for Native {
    fn name(&self) -> String {
        "native".into()
    }

    fn alert(&self, title: &str, message: &str) {
        message_box(title, message, MB_OK | MB_ICONINFORMATION);
    }

    fn confirm(&self, title: &str, message: &str) -> bool {
        message_box(title, message, MB_OKCANCEL | MB_ICONQUESTION) == IDOK
    }

    fn prompt(&self, title: &str, message: &str, def: &str) -> Option<String> {
        notify::watching(title, message, || dialogs::prompt(title, message, def))
    }

    fn open_files(&self, o: &PickerOptions) -> Vec<String> {
        file_dialog(false, false, o).unwrap_or_else(|e| {
            self.show_error(&o.title, &e.to_string());
            Vec::new()
        })
    }

    fn save_file(&self, o: &PickerOptions) -> Option<String> {
        match file_dialog(true, false, o) {
            Ok(p) => p.into_iter().next(),
            Err(e) => {
                self.show_error(&o.title, &e.to_string());
                None
            }
        }
    }

    fn pick_directory(&self, o: &PickerOptions) -> Option<String> {
        match file_dialog(false, true, o) {
            Ok(p) => p.into_iter().next(),
            Err(e) => {
                self.show_error(&o.title, &e.to_string());
                None
            }
        }
    }

    fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress> {
        match NativeProgress::start(o) {
            Some(p) => Arc::new(p),
            // Fall back to a console indicator so scripts keep working.
            None => Console::new().progress(o),
        }
    }

    /// Shows the error in a dialog whose text can be selected and copied,
    /// falling back to a message box.
    fn show_error(&self, title: &str, message: &str) {
        if !notify::watching(title, message, || dialogs::error_dialog(title, message)) {
            message_box(title, message, MB_OK | MB_ICONERROR);
        }
    }

    fn show_in_folder(&self, paths: &[String]) -> Result<()> {
        shell_show_in_folder(paths)
    }

    fn open_path(&self, target: &str) -> Result<()> {
        shell_open(target)
    }

    fn open_window(&self, o: WindowOptions) -> Result<Arc<dyn Window>> {
        webview::open(o)
    }

    fn supports_windows(&self) -> bool {
        true
    }

    fn form(&self, _o: &FormOptions) -> Option<Map<String, Value>> {
        None
    }

    fn table(&self, _o: &TableOptions) -> Option<Vec<i64>> {
        None
    }

    fn choose(&self, title: &str, message: &str, choices: &[Choice]) -> Option<i64> {
        Some(dialogs::choose(title, message, choices))
    }

    fn notify(&self, title: &str, message: &str) {
        notify::show(title, message);
    }

    fn print_pdf(&self, job: PdfJob, done: PdfDone) {
        webview::print_pdf(job, done);
    }
}

/// A progress indicator (IProgressDialog).
struct NativeProgress {
    st: Mutex<ProgressState>,
}

struct ProgressState {
    dlg: IProgressDialog,
    total: i64,
    last: Option<Instant>,
    closed: bool,
}

// Only used on the UI thread (see the module documentation).
unsafe impl Send for NativeProgress {}
unsafe impl Sync for NativeProgress {}

impl NativeProgress {
    fn start(o: &ProgressOptions) -> Option<NativeProgress> {
        unsafe {
            let dlg: IProgressDialog = CoCreateInstance(&CLSID_ProgressDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            progress_opened();
            let _ = dlg.SetTitle(&hstr(&o.title));
            let mut flags = if o.total <= 0 { PROGDLG_MARQUEEPROGRESS | PROGDLG_NOTIME } else { PROGDLG_AUTOTIME };
            if !o.cancellable {
                flags |= PROGDLG_NOCANCEL;
            }
            let _ = dlg.StartProgressDialog(owner(), None, flags, None);
            let _ = dlg.Timer(PDTIMER_RESET, None);
            let p = NativeProgress { st: Mutex::new(ProgressState { dlg, total: o.total, last: None, closed: false }) };
            p.set_lines(&p.st.lock().unwrap(), 0, &o.message);
            Some(p)
        }
    }

    fn set_line(st: &ProgressState, n: u32, s: &str) {
        unsafe {
            let _ = st.dlg.SetLine(n, &hstr(s), true, None);
        }
    }

    fn set_lines(&self, st: &ProgressState, done: i64, message: &str) {
        Self::set_line(st, 1, message);
        if st.total > 0 {
            Self::set_line(st, 2, &format!("{done} / {}", st.total));
            unsafe {
                let _ = st.dlg.SetProgress(done.max(0) as u32, st.total as u32);
            }
        } else if done > 0 {
            Self::set_line(st, 2, &done.to_string());
        }
    }
}

impl Progress for NativeProgress {
    fn update(&self, done: i64, total: i64, message: &str) {
        let mut st = self.st.lock().unwrap();
        if st.closed {
            return;
        }
        if total > 0 {
            st.total = total;
        }
        if st.last.is_some_and(|l| l.elapsed() < Duration::from_millis(50)) && done < st.total {
            return;
        }
        st.last = Some(Instant::now());
        self.set_lines(&st, done, message);
    }

    fn cancelled(&self) -> bool {
        let st = self.st.lock().unwrap();
        if st.closed {
            return false;
        }
        unsafe { st.dlg.HasUserCancelled().as_bool() || interrupted() }
    }

    fn close(&self) {
        let mut st = self.st.lock().unwrap();
        if st.closed {
            return;
        }
        st.closed = true;
        unsafe {
            let _ = st.dlg.StopProgressDialog();
        }
        progress_closed();
    }
}

/// Opens Explorer with the files selected. Files in different folders open
/// one Explorer window per folder.
pub(crate) fn show_in_folder(groups: &[(std::path::PathBuf, Vec<std::path::PathBuf>)]) -> Result<()> {
    unsafe {
        // COM must be initialised on this thread (no-op if it already is).
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        for (dir, files) in groups {
            let folder = ILCreateFromPathW(&hstr(&dir.to_string_lossy()));
            if folder.is_null() {
                return Err(err!("shell.cannotOpenFolder", dir.display()));
            }
            let mut items = Vec::new();
            let mut children = Vec::new();
            for f in files {
                let it = ILCreateFromPathW(&hstr(&f.to_string_lossy()));
                if !it.is_null() {
                    items.push(it);
                    children.push(ILFindLastID(it) as *const _);
                }
            }
            let r = SHOpenFolderAndSelectItems(folder, if children.is_empty() { None } else { Some(&children) }, 0);
            for it in items {
                ILFree(Some(it));
            }
            ILFree(Some(folder));
            r.map_err(|e| Error::msg(format!("SHOpenFolderAndSelectItems failed: {e}")))?;
        }
        Ok(())
    }
}

/// Opens a file, folder or URL with its default application.
pub(crate) fn shell_execute(target: &str) -> Result<()> {
    unsafe {
        let r = ShellExecuteW(None, w!("open"), &hstr(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if r.0 as isize <= 32 {
            return Err(err!("shell.cannotOpen", target, r.0 as isize));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
