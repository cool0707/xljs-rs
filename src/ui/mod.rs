//! The user-interaction layer used by scripts (dialogs, file pickers,
//! progress, HTML windows) and its implementations: native Windows dialogs,
//! a console fallback, and a scripted mock for automated tests.

mod builtin;
mod console;
mod console_form;
mod icon;
pub mod mock;
mod mock_app;
#[cfg(windows)]
mod native;
mod shell;
mod thread;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::meta::AssetFs;

pub use builtin::{BUILTIN_PREFIX, BuiltinAssets, PAGE_SCRIPT, builtin_file};
pub(crate) use builtin::dayjs_files;
pub use console::Console;
pub use icon::{check_icon, icon_image};
pub use mock::Mock;
pub use mock_app::AppStep;
pub use shell::{shell_open, shell_show_in_folder};
pub use thread::run;

/// One entry of a file type filter in a picker.
#[derive(Debug, Clone, Default)]
pub struct FileFilter {
    pub description: String,
    pub patterns: Vec<String>, // e.g. "*.xlsx"
}

/// Configures file and directory pickers.
#[derive(Debug, Clone, Default)]
pub struct PickerOptions {
    pub title: String,
    pub multiple: bool,
    pub filters: Vec<FileFilter>,
    /// The directory to start in ("" = default).
    pub start_in: String,
    /// The suggested name (save picker only).
    pub suggested_name: String,
}

/// Configures a progress indicator.
#[derive(Debug, Clone, Default)]
pub struct ProgressOptions {
    pub title: String,
    /// 0 = indeterminate
    pub total: i64,
    pub message: String,
    pub cancellable: bool,
}

/// A live progress indicator.
pub trait Progress: Send + Sync {
    /// Sets the number of completed items, the total (0 keeps the current
    /// total) and the message line.
    fn update(&self, done: i64, total: i64, message: &str);
    /// Whether the user asked to cancel.
    fn cancelled(&self) -> bool;
    /// Removes the indicator. It is safe to call more than once.
    fn close(&self);
}

/// The URL under which a window serves WindowOptions::assets.
pub const ASSET_ORIGIN: &str = "https://app.xljs/";

pub type OnMessage = Arc<dyn Fn(String, Vec<String>) + Send + Sync>;
pub type OnClosed = Arc<dyn Fn() + Send + Sync>;
pub type Trace = Arc<dyn Fn(String) + Send + Sync>;

/// Configures an HTML window.
#[derive(Clone, Default)]
pub struct WindowOptions {
    pub title: String,
    /// The client size in logical pixels (0 = default).
    pub width: i64,
    pub height: i64,
    /// Keeps the window at its size: no resizing border and no maximize
    /// button.
    pub fixed_size: bool,
    /// The first page; "" shows an empty page. Pages under ASSET_ORIGIN are
    /// served from assets.
    pub url: String,
    pub assets: Option<Arc<dyn AssetFs>>,
    /// Enables the developer tools (F12) and the context menu; dev_tools
    /// also opens them with the window.
    pub debug: bool,
    pub dev_tools: bool,
    /// Serves the pages cross-origin isolated (COOP and COEP headers), so
    /// that they can use SharedArrayBuffer and Atomics.wait (the debugger).
    pub isolated: bool,
    /// An .ico file or a PNG image for the title bar and taskbar (empty =
    /// the exe's icon). See check_icon.
    pub icon: Vec<u8>,
    /// Receives the messages posted by the page, with the paths of the
    /// files the page attached. on_closed is called once after the window
    /// has closed. Both may be called on any thread and must not block.
    pub on_message: Option<OnMessage>,
    pub on_closed: Option<OnClosed>,
    /// Receives what happens to the page that the page cannot report
    /// itself (tests). Called on the UI thread.
    pub trace: Option<Trace>,
}

/// The environment variable that overrides the WebView2 user data folder
/// (default %LOCALAPPDATA%\xljs\WebView2).
pub const WEBVIEW2_DATA_ENV: &str = "XLJS_WEBVIEW2_DATA";

/// An open HTML window.
pub trait Window: Send + Sync {
    /// Sends a message to the page.
    fn post(&self, msg: String);
    /// Closes the window. It is safe to call more than once.
    fn close(&self);
}

/// The set of interactions available to scripts. Implementations are
/// called from one thread only (see run).
pub trait Ui: Send + Sync {
    fn name(&self) -> String;
    fn alert(&self, title: &str, message: &str);
    fn confirm(&self, title: &str, message: &str) -> bool;
    /// The entered text, None when cancelled.
    fn prompt(&self, title: &str, message: &str, def: &str) -> Option<String>;
    /// The selected paths (empty when cancelled).
    fn open_files(&self, o: &PickerOptions) -> Vec<String>;
    /// The chosen path, None when cancelled.
    fn save_file(&self, o: &PickerOptions) -> Option<String>;
    /// The chosen directory, None when cancelled.
    fn pick_directory(&self, o: &PickerOptions) -> Option<String>;
    fn progress(&self, o: &ProgressOptions) -> Arc<dyn Progress>;
    /// Reports a fatal script error to the user.
    fn show_error(&self, title: &str, message: &str);
    /// Opens the file manager with the given files selected.
    fn show_in_folder(&self, paths: &[String]) -> Result<()>;
    /// Opens a file, folder or URL with its default application.
    fn open_path(&self, target: &str) -> Result<()>;
    /// Opens an HTML window (WebView2). It fails when the UI cannot show
    /// one (console) or the WebView2 runtime is missing.
    fn open_window(&self, o: WindowOptions) -> Result<Arc<dyn Window>>;
    /// Whether forms and tables are shown as HTML windows (driven by the
    /// script through open_window) instead of form and table.
    fn supports_windows(&self) -> bool;
    /// Asks for the values of a form, by field name; None when cancelled.
    fn form(&self, o: &FormOptions) -> Option<serde_json::Map<String, serde_json::Value>>;
    /// Shows rows. With o.select it returns the indices of the chosen rows,
    /// None when cancelled.
    fn table(&self, o: &TableOptions) -> Option<Vec<i64>>;
    /// Asks which of a few actions to take (the start screen); the index of
    /// the chosen option, -1 when cancelled. None when the UI cannot.
    fn choose(&self, _title: &str, _message: &str, _choices: &[Choice]) -> Option<i64> {
        None
    }
    /// Shows a notification (on Windows, the one at the bottom right of
    /// the screen) and returns without waiting for it.
    fn notify(&self, _title: &str, _message: &str) {}
}

/// A form for UIs without HTML windows. Values are strings for every field
/// type except checkbox (bool), files (string array) and table (the
/// indices of the selected rows); empty means no value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FormOptions {
    pub title: String,
    pub description: String,
    pub fields: Vec<FormField>,
    /// A message about the form as a whole (from a previous attempt).
    pub error: String,
    /// The label of the submit button ("" = none) and those of the other
    /// buttons. With buttons, the values returned name the one pressed in
    /// "$button": its index, or null for submit.
    pub submit: String,
    pub buttons: Vec<String>,
}

/// One input of a form.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FormField {
    pub name: String,
    pub label: String,
    /// text, password, number, date, month, time, select, radio, checkbox,
    /// textarea, file, files, folder, saveFile, table
    #[serde(rename = "type")]
    pub kind: String,
    pub options: Vec<FormOption>,
    pub required: bool,
    pub placeholder: String,
    pub help: String,
    pub accept: Vec<String>,
    pub value: serde_json::Value,
    pub error: String,
    /// The label of the tab the field is on ("" = above the tabs).
    pub tab: String,
    /// The heading of the group the field is in ("" = not in a group).
    pub group: String,
    /// The description shown under the group's heading.
    pub group_help: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub select: String,
    pub editable: Vec<FormCellColumn>,
    pub cell_errors: Vec<FormCellError>,
}

/// An editable column of a table field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FormCellColumn {
    pub index: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub options: Vec<FormOption>,
}

/// A cell of a table field that does not pass its checks (col -1: the row
/// as a whole).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FormCellError {
    pub row: i64,
    pub col: i64,
    pub message: String,
}

/// A choice of a select or radio field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FormOption {
    pub value: String,
    pub label: String,
}

pub const SELECT_NONE: &str = "";
pub const SELECT_SINGLE: &str = "single";
pub const SELECT_MULTIPLE: &str = "multiple";

/// A table for UIs without HTML windows. Cells are already formatted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TableOptions {
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub select: String,
}

/// One of the options of Ui::choose.
#[derive(Debug, Clone, Default)]
pub struct Choice {
    pub label: String,
    /// A line of explanation under the label.
    pub note: String,
    /// An .ico file or a PNG image (empty = the default arrow).
    pub icon: Vec<u8>,
}

static INTERRUPTS: AtomicI32 = AtomicI32::new(0);
static AUTO_NOTIFY: AtomicBool = AtomicBool::new(true);

/// Sets whether the native UI shows a notification when a dialog or a
/// window opens while the user is working in another application (the
/// dialog then cannot come to the front by itself).
pub fn set_auto_notify(on: bool) {
    AUTO_NOTIFY.store(on, Ordering::SeqCst);
}

/// See set_auto_notify.
pub fn auto_notify() -> bool {
    AUTO_NOTIFY.load(Ordering::SeqCst)
}
static ACTIVE_PROGRESS: AtomicI32 = AtomicI32::new(0);

/// Records a Ctrl+C press. It returns the total number of presses so far.
pub fn notify_interrupt() -> i32 {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst) + 1
}

/// Whether Ctrl+C was pressed at least once.
pub fn interrupted() -> bool {
    INTERRUPTS.load(Ordering::SeqCst) > 0
}

/// Whether any progress indicator is open (the first Ctrl+C then only
/// requests cancellation).
pub fn progress_active() -> bool {
    ACTIVE_PROGRESS.load(Ordering::SeqCst) > 0
}

pub(crate) fn progress_opened() {
    ACTIVE_PROGRESS.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn progress_closed() {
    ACTIVE_PROGRESS.fetch_sub(1, Ordering::SeqCst);
}

/// The native UI. It is only available on Windows.
pub fn new_native() -> Result<Arc<dyn Ui>> {
    #[cfg(windows)]
    {
        native::Native::new()
    }
    #[cfg(not(windows))]
    {
        Err(crate::err!("ui.nativeUnavailable"))
    }
}

/// Whether new_native can succeed.
pub fn native_available() -> bool {
    #[cfg(windows)]
    {
        native::available()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The version of the WebView2 runtime ("" when it is missing).
pub fn webview2_version() -> String {
    #[cfg(windows)]
    {
        native::webview2_version()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

/// Quotes s like Go's strconv.Quote (the mock's log format).
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Formats strings like Go's %q of a []string: `["a" "b"]`.
pub fn quote_list(v: &[String]) -> String {
    let items: Vec<String> = v.iter().map(|s| quote(s)).collect();
    format!("[{}]", items.join(" "))
}

#[cfg(test)]
mod tests;
