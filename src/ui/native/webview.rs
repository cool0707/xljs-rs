//! HTML windows (WebView2). Pages under ASSET_ORIGIN are served from the
//! window's assets; the page runtime (web/xljs.js) is injected into every
//! page; navigation away from the app opens the default browser instead.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use base64::Engine as _;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::Shell::{ExtractIconExW, SHCreateMemStream};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCWSTR, PWSTR, w};

use super::{WINDOWS, hstr, loader};
use crate::ui::{
    ASSET_ORIGIN, BUILTIN_PREFIX, BuiltinAssets, PAGE_SCRIPT, PdfDone, PdfJob, TITLE_BAR_HEIGHT, Window, WindowOptions, icon_image, title_bar_script,
};
use crate::{Error, Result, err, i18n};

/// Where users get the WebView2 runtime.
pub const WEBVIEW2_DOWNLOAD_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

const DEFAULT_WIDTH: i64 = 960;
const DEFAULT_HEIGHT: i64 = 640;

/// Host → page messages start with this, so that page scripts can tell them
/// from anything else they receive.
const HOST_PREFIX: &str = "\u{2}";

/// Page → host messages that the window handles itself (the custom title
/// bar, web/titlebar.js): "minimize", "maximize" (toggles), "close",
/// "drag" (moves the window with the mouse) and "state" (asks for
/// __xljsTitleBarState).
const WINDOW_COMMAND_PREFIX: &str = "\u{1}window:";

/// The version of the installed WebView2 runtime ("" when missing).
pub(crate) fn webview2_version() -> String {
    loader::find_installed().map(|(_, v)| v).unwrap_or_default()
}

struct WinState {
    o: WindowOptions,
    controller: RefCell<Option<ICoreWebView2Controller>>,
    core: RefCell<Option<ICoreWebView2>>,
    env: RefCell<Option<ICoreWebView2Environment>>,
    icons: [HICON; 2], // big, small (from o.icon; destroyed with the window)
    closed: RefCell<bool>,
    active: Cell<bool>,
}

thread_local! {
    static STATES: RefCell<HashMap<isize, Rc<WinState>>> = RefCell::new(HashMap::new());
}

fn state(hwnd: HWND) -> Option<Rc<WinState>> {
    STATES.with(|s| s.borrow().get(&(hwnd.0 as isize)).cloned())
}

/// The handle of a window (all state lives on the UI thread).
struct WebWindow {
    hwnd: isize,
}

impl WebWindow {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }
}

impl Window for WebWindow {
    fn post(&self, msg: String) {
        let core = state(self.hwnd()).and_then(|st| st.core.borrow().clone());
        if let Some(core) = core {
            unsafe {
                let _ = core.PostWebMessageAsString(&hstr(&format!("{HOST_PREFIX}{msg}")));
            }
        }
    }

    fn close(&self) {
        if let Some(st) = state(self.hwnd())
            && !*st.closed.borrow()
        {
            unsafe {
                let _ = DestroyWindow(self.hwnd());
            }
        }
    }
}

static CLASS: OnceLock<bool> = OnceLock::new();
static EXE_ICONS: OnceLock<(isize, isize)> = OnceLock::new();

/// The exe's icons (big, small).
pub(super) fn exe_icons() -> (HICON, HICON) {
    let (a, b) = *EXE_ICONS.get_or_init(|| unsafe {
        let mut big = HICON::default();
        let mut small = HICON::default();
        if let Ok(exe) = std::env::current_exe() {
            ExtractIconExW(&hstr(&exe.to_string_lossy()), 0, Some(&mut big), Some(&mut small), 1);
        }
        (big.0 as isize, small.0 as isize)
    });
    (HICON(a as *mut _), HICON(b as *mut _))
}

/// An icon of size pixels from an .ico file or a PNG image.
pub(super) fn icon_from_bytes(data: &[u8], size: u32) -> Option<HICON> {
    let img = icon_image(data, size as usize)?;
    unsafe { CreateIconFromResourceEx(img, true, 0x00030000, size as i32, size as i32, LR_DEFAULTCOLOR).ok() }
}

pub(super) fn icon_for_metric(data: &[u8], metric: SYSTEM_METRICS_INDEX) -> HICON {
    let size = unsafe { GetSystemMetrics(metric) };
    icon_from_bytes(data, if size > 0 { size as u32 } else { 32 }).unwrap_or_default()
}

fn register_class() {
    CLASS.get_or_init(|| unsafe {
        let (big, small) = exe_icons();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            hIcon: big,
            hIconSm: small,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("xljsWindow"),
            ..Default::default()
        };
        RegisterClassExW(&wc) != 0
    });
}

pub(super) fn open(o: WindowOptions) -> Result<Arc<dyn Window>> {
    let hwnd = open_hwnd(o)?;
    Ok(Arc::new(WebWindow { hwnd: hwnd.0 as isize }))
}

fn open_hwnd(o: WindowOptions) -> Result<HWND> {
    if webview2_version().is_empty() {
        return Err(err!("ui.webview2Missing", WEBVIEW2_DOWNLOAD_URL));
    }
    register_class();
    let width = if o.width > 0 { o.width } else { DEFAULT_WIDTH };
    let mut height = if o.height > 0 { o.height } else { DEFAULT_HEIGHT };
    if o.custom_title_bar {
        // The page's size is below the bar.
        height += TITLE_BAR_HEIGHT;
    }
    let style = if o.hidden {
        WS_POPUP
    } else if o.fixed_size {
        WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
    } else {
        WS_OVERLAPPEDWINDOW
    };
    // A hidden window is shown but nearly transparent, so that the page
    // renders (requestAnimationFrame) as in a visible window.
    let ex_style = if o.hidden { WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT } else { Default::default() };
    unsafe {
        let mut dpi = GetDpiForSystem();
        if dpi == 0 {
            dpi = 96;
        }
        let mut r = RECT { left: 0, top: 0, right: (width * dpi as i64 / 96) as i32, bottom: (height * dpi as i64 / 96) as i32 };
        let _ = AdjustWindowRectEx(&mut r, style, false, Default::default());
        if o.custom_title_bar {
            // No caption or top border (see WM_NCCALCSIZE).
            r.top = 0;
        }
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as *mut _), Default::default());
        let x = work.left + 0.max((work.right - work.left - ww) / 2);
        let y = work.top + 0.max((work.bottom - work.top - wh) / 2);
        let hwnd = CreateWindowExW(
            ex_style,
            w!("xljsWindow"),
            &hstr(&o.title),
            style,
            x,
            y,
            ww,
            wh,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
        .map_err(|e| Error::msg(e.to_string()))?;
        let icons = if o.icon.is_empty() {
            [HICON::default(), HICON::default()]
        } else {
            [icon_for_metric(&o.icon, SM_CXICON), icon_for_metric(&o.icon, SM_CXSMICON)]
        };
        let (eb, es) = exe_icons();
        for (i, which) in [ICON_BIG, ICON_SMALL].into_iter().enumerate() {
            let icon = if !icons[i].is_invalid() { icons[i] } else if i == 0 { eb } else { es };
            if !icon.is_invalid() {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(icon.0 as isize)));
            }
        }
        let title = o.title.clone();
        let st = Rc::new(WinState {
            o,
            controller: RefCell::new(None),
            core: RefCell::new(None),
            env: RefCell::new(None),
            icons,
            closed: RefCell::new(false),
            active: Cell::new(true),
        });
        STATES.with(|s| s.borrow_mut().insert(hwnd.0 as isize, st.clone()));
        if st.o.custom_title_bar {
            // WM_NCCALCSIZE again, now that the state is known.
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        if st.o.hidden {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
        if let Err(e) = embed(hwnd, &st) {
            // The caller learns from the error, not from on_closed.
            *st.closed.borrow_mut() = true;
            STATES.with(|s| s.borrow_mut().remove(&(hwnd.0 as isize)));
            let _ = DestroyWindow(hwnd);
            return Err(e);
        }
        if st.o.hidden {
            return Ok(hwnd);
        }
        WINDOWS.with(|w| w.borrow_mut().push(hwnd));
        // Not cancelled: the window stays open.
        super::notify::watch(&if title.is_empty() { "xljs".to_string() } else { title }, &crate::t!("ui.waiting"));
        Ok(hwnd)
    }
}

/// Serves the built-in pages to the print window.
struct PrintAssets;

impl crate::meta::AssetFs for PrintAssets {
    fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        match name.strip_prefix(BUILTIN_PREFIX) {
            Some(rest) => crate::meta::AssetFs::read(&BuiltinAssets, rest),
            None => Err(std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string())),
        }
    }
    fn files(&self, _root: &str) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// Calls a DevTools protocol method of the page; f gets the result JSON.
fn devtools(core: &ICoreWebView2, method: &str, params: &str, f: impl FnOnce(windows::core::Result<()>, String) + 'static) {
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, json: String| {
        f(hr, json);
        Ok(())
    }));
    unsafe {
        if let Err(e) = core.CallDevToolsProtocolMethod(&hstr(method), &hstr(params), &handler) {
            // The handler is not called.
            let _ = e;
        }
    }
}

/// See Ui::print_pdf: web/print.html in a hidden window puts the document
/// in place and draws its charts, then Page.printToPDF (the page sizes and
/// margins come from the document's @page rules) writes the file.
pub(super) fn print_pdf(job: PdfJob, done: PdfDone) {
    let done = Arc::new(Mutex::new(Some(done)));
    // Calls done once; true if this call did.
    let finish: Arc<dyn Fn(Result<()>) -> bool + Send + Sync> = Arc::new(move |r| {
        let d = done.lock().unwrap().take();
        match d {
            Some(d) => {
                d(r);
                true
            }
            None => false,
        }
    });
    let hwnd_id = Arc::new(AtomicIsize::new(0));
    let (f1, f2, h1) = (finish.clone(), finish.clone(), hwnd_id.clone());
    let job = Arc::new(job);
    let on_message: crate::ui::OnMessage = Arc::new(move |msg: String, _| {
        let hwnd = HWND(h1.load(Ordering::SeqCst) as *mut _);
        let Some(core) = state(hwnd).and_then(|st| st.core.borrow().clone()) else { return };
        let Ok(m) = serde_json::from_str::<serde_json::Value>(&msg) else { return };
        let close = move || unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        };
        match m.get("type").and_then(|t| t.as_str()) {
            Some("ready") => {
                // Charts in their light colors whatever the system's theme.
                devtools(&core, "Emulation.setEmulatedMedia", r#"{"features":[{"name":"prefers-color-scheme","value":"light"}]}"#, |_, _| {});
                let charts: serde_json::Value = serde_json::from_str(&job.charts).unwrap_or(serde_json::json!([]));
                let texts: serde_json::Value = serde_json::from_str(&job.texts).unwrap_or(serde_json::json!({}));
                let init = serde_json::json!({ "type": "init", "html": job.html, "charts": charts, "texts": texts, "lang": i18n::lang() });
                unsafe {
                    let _ = core.PostWebMessageAsString(&hstr(&format!("{HOST_PREFIX}{init}")));
                }
            }
            Some("rendered") => {
                let (f, path) = (f1.clone(), job.path.clone());
                let params = r#"{"printBackground":true,"preferCSSPageSize":true,"displayHeaderFooter":false,"marginTop":0,"marginBottom":0,"marginLeft":0,"marginRight":0,"transferMode":"ReturnAsBase64"}"#;
                devtools(&core, "Page.printToPDF", params, move |hr, json| {
                    let r = (|| -> Result<()> {
                        hr.map_err(webview_err)?;
                        let v: serde_json::Value = serde_json::from_str(&json)?;
                        let data = v.get("data").and_then(|d| d.as_str()).ok_or_else(|| webview_err(&json))?;
                        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(webview_err)?;
                        std::fs::write(&path, bytes).map_err(|e| Error::Path { op: "open".into(), path: path.clone(), source: e })?;
                        Ok(())
                    })();
                    f(r);
                    close();
                });
            }
            Some("error") => {
                let message = m.get("message").and_then(|t| t.as_str()).unwrap_or("").to_string();
                f1(Err(err!("pdf.failed", message)));
                close();
            }
            _ => {}
        }
    });
    let on_closed: crate::ui::OnClosed = Arc::new(move || {
        f2(Err(err!("pdf.failed", "closed")));
    });
    let o = WindowOptions {
        title: "xljs PDF".into(),
        width: 1000,
        height: 800,
        url: format!("{ASSET_ORIGIN}{BUILTIN_PREFIX}print.html"),
        assets: Some(Arc::new(PrintAssets)),
        hidden: true,
        on_message: Some(on_message),
        on_closed: Some(on_closed),
        ..Default::default()
    };
    match open_hwnd(o) {
        Ok(h) => {
            hwnd_id.store(h.0 as isize, Ordering::SeqCst);
            // A page that never answers (it cannot load, a chart hangs)
            // must not keep the script waiting.
            let id = h.0 as isize;
            std::thread::spawn(move || {
                std::thread::sleep(PRINT_TIMEOUT);
                // Only while the job is open: the handle may name another window later.
                if finish(Err(err!("pdf.failed", "timeout"))) {
                    unsafe {
                        let _ = PostMessageW(Some(HWND(id as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            });
        }
        Err(e) => {
            finish(Err(e));
        }
    }
}

/// How long print_pdf waits for the page.
const PRINT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn webview_err(e: impl std::fmt::Display) -> Error {
    err!("ui.webviewError", e)
}

fn data_folder() -> String {
    if let Ok(d) = std::env::var(crate::ui::WEBVIEW2_DATA_ENV)
        && !d.is_empty()
    {
        return d;
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
    std::path::Path::new(&base).join("xljs").join("WebView2").to_string_lossy().into_owned()
}

fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

fn embed(hwnd: HWND, st: &Rc<WinState>) -> Result<()> {
    unsafe {
        // The environment.
        let (tx, rx) = mpsc::channel();
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr: windows::core::Result<()>, env| {
            let _ = tx.send(hr.map(|_| env));
            Ok(())
        }));
        let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();
        loader::create_environment(&data_folder(), &options, &handler).map_err(webview_err)?;
        let env = wait_with_pump(rx).map_err(webview_err)?.map_err(webview_err)?.ok_or_else(|| webview_err("no environment"))?;

        // The controller.
        let (tx, rx) = mpsc::channel();
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |hr: windows::core::Result<()>, c| {
            let _ = tx.send(hr.map(|_| c));
            Ok(())
        }));
        env.CreateCoreWebView2Controller(hwnd, &handler).map_err(webview_err)?;
        let controller = wait_with_pump(rx).map_err(webview_err)?.map_err(webview_err)?.ok_or_else(|| webview_err("no controller"))?;
        let core = controller.CoreWebView2().map_err(webview_err)?;
        *st.env.borrow_mut() = Some(env.clone());
        *st.controller.borrow_mut() = Some(controller.clone());
        *st.core.borrow_mut() = Some(core.clone());

        if let Ok(s) = core.Settings() {
            let _ = s.SetAreDevToolsEnabled(st.o.debug);
            let _ = s.SetAreDefaultContextMenusEnabled(st.o.debug);
            let _ = s.SetIsStatusBarEnabled(false);
        }
        // CSS app-region: drag for the custom title bar (WebView2 1.0.2420+;
        // older runtimes move the window on the page's request).
        let non_client = st.o.custom_title_bar
            && core.Settings().ok().and_then(|s| s.cast::<ICoreWebView2Settings9>().ok()).is_some_and(|s| s.SetIsNonClientRegionSupportEnabled(true).is_ok());

        // Messages from the page, with the paths of attached files.
        let on_message = st.o.on_message.clone();
        let hwnd_id = hwnd.0 as isize;
        let mut token = 0i64;
        core.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                let Some(args) = args else { return Ok(()) };
                let mut p = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut p).is_err() {
                    return Ok(());
                }
                let msg = take_pwstr(p);
                if let Some(cmd) = msg.strip_prefix(WINDOW_COMMAND_PREFIX) {
                    window_command(HWND(hwnd_id as *mut _), cmd);
                    return Ok(());
                }
                let files = attached_files(&args);
                if let Some(f) = &on_message {
                    f(msg, files);
                }
                Ok(())
            })),
            &mut token,
        )
        .map_err(webview_err)?;

        // The app's files.
        core.AddWebResourceRequestedFilter(&hstr(&format!("{ASSET_ORIGIN}*")), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL).map_err(webview_err)?;
        let (assets, isolated, envc) = (st.o.assets.clone(), st.o.isolated, env.clone());
        core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args: Option<ICoreWebView2WebResourceRequestedEventArgs>| {
                if let Some(args) = args {
                    serve(&envc, &args, assets.as_deref(), isolated);
                }
                Ok(())
            })),
            &mut token,
        )
        .map_err(webview_err)?;

        // Navigation stays inside the app; links to the web open the
        // default browser, popups never open.
        core.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(|_, args: Option<ICoreWebView2NavigationStartingEventArgs>| {
                let Some(args) = args else { return Ok(()) };
                let mut p = PWSTR::null();
                if args.Uri(&mut p).is_err() {
                    return Ok(());
                }
                let uri = take_pwstr(p);
                if inside(&uri) {
                    return Ok(());
                }
                let _ = args.SetCancel(true);
                if external(&uri) {
                    let _ = super::shell_execute(&uri);
                }
                Ok(())
            })),
            &mut token,
        )
        .map_err(webview_err)?;
        core.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args: Option<ICoreWebView2NewWindowRequestedEventArgs>| {
                let Some(args) = args else { return Ok(()) };
                let _ = args.SetHandled(true);
                let mut p = PWSTR::null();
                if args.Uri(&mut p).is_ok() {
                    let uri = take_pwstr(p);
                    if external(&uri) {
                        let _ = super::shell_execute(&uri);
                    }
                }
                Ok(())
            })),
            &mut token,
        )
        .map_err(webview_err)?;
        if let Some(trace) = st.o.trace.clone() {
            let t2 = trace.clone();
            let _ = core.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |_, _| {
                    trace("navigation completed".into());
                    Ok(())
                })),
                &mut token,
            );
            let _ = core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, args: Option<ICoreWebView2ProcessFailedEventArgs>| {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    if let Some(a) = args {
                        let _ = a.ProcessFailedKind(&mut kind);
                    }
                    t2(format!("process failed (kind {})", kind.0));
                    Ok(())
                })),
                &mut token,
            );
        }

        // The page runtime, in every page.
        let _ = core.AddScriptToExecuteOnDocumentCreated(&hstr(PAGE_SCRIPT), None);
        if st.o.custom_title_bar {
            let _ = core.AddScriptToExecuteOnDocumentCreated(&hstr(&title_bar_script(&st.o, non_client)), None);
        }

        resize(hwnd, &controller);
        focus(hwnd, &controller);
        if st.o.url.is_empty() {
            let _ = core.NavigateToString(w!("<!doctype html><title></title>"));
        } else {
            core.Navigate(&hstr(&st.o.url)).map_err(webview_err)?;
        }
        if st.o.debug && st.o.dev_tools {
            let _ = core.OpenDevToolsWindow();
        }
    }
    Ok(())
}

/// The paths of the files the page attached
/// (chrome.webview.postMessageWithAdditionalObjects); "" where a path is not
/// available, so that the indices match.
fn attached_files(args: &ICoreWebView2WebMessageReceivedEventArgs) -> Vec<String> {
    let Ok(args2) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>() else { return Vec::new() };
    unsafe {
        let Ok(objs) = args2.AdditionalObjects() else { return Vec::new() };
        let mut count = 0u32;
        if objs.Count(&mut count).is_err() {
            return Vec::new();
        }
        (0..count)
            .map(|i| {
                let Ok(v) = objs.GetValueAtIndex(i) else { return String::new() };
                let Ok(f) = v.cast::<ICoreWebView2File>() else { return String::new() };
                let mut p = PWSTR::null();
                if f.Path(&mut p).is_err() {
                    return String::new();
                }
                take_pwstr(p)
            })
            .collect()
    }
}

fn content_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len() + 0
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Answers a request for a file under ASSET_ORIGIN.
fn serve(env: &ICoreWebView2Environment, args: &ICoreWebView2WebResourceRequestedEventArgs, assets: Option<&dyn crate::meta::AssetFs>, isolated: bool) {
    unsafe {
        let Ok(req) = args.Request() else { return };
        let mut p = PWSTR::null();
        if req.Uri(&mut p).is_err() {
            return;
        }
        let uri = take_pwstr(p);
        let Some(rest) = uri.strip_prefix(ASSET_ORIGIN) else { return };
        let mut name = rest.split(['?', '#']).next().unwrap_or("").to_string();
        name = percent_decode(&name);
        if name.is_empty() || name.ends_with('/') {
            name.push_str("index.html");
        }
        let body = match assets {
            Some(a) if crate::meta::assets_valid_path(&name) => a.read(&name).ok(),
            _ => None,
        };
        let (status, reason, ctype, body) = match body {
            Some(b) => (200, "OK", content_type(&name), b),
            None => (404, "Not Found", "text/plain; charset=utf-8", format!("404 Not Found: {name}").into_bytes()),
        };
        let mut headers = format!("Content-Type: {ctype}\r\nCache-Control: no-cache");
        if isolated {
            headers.push_str("\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp");
        }
        let stream = SHCreateMemStream(Some(&body));
        if let Ok(resp) = env.CreateWebResourceResponse(stream.as_ref(), status, &hstr(reason), &hstr(&headers)) {
            let _ = args.SetResponse(&resp);
        }
    }
}

fn inside(uri: &str) -> bool {
    uri.starts_with(ASSET_ORIGIN) || uri.starts_with("about:") || uri.starts_with("data:")
}

fn external(uri: &str) -> bool {
    let u = uri.to_lowercase();
    u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")
}

/// Handles a WINDOW_COMMAND_PREFIX message of the page.
fn window_command(hwnd: HWND, cmd: &str) {
    let Some(st) = state(hwnd) else { return };
    unsafe {
        match cmd {
            "minimize" => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            "maximize" if !st.o.fixed_size => {
                let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
            }
            "close" => {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            "drag" => {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ReleaseCapture();
                let pos = ((pt.y as u16 as u32) << 16) | pt.x as u16 as u32;
                let _ = PostMessageW(Some(hwnd), WM_NCLBUTTONDOWN, WPARAM(HTCAPTION as usize), LPARAM(pos as i32 as isize));
            }
            "state" => sync_title_bar(hwnd, &st),
            _ => {}
        }
    }
}

/// Tells the custom title bar whether the window is maximized and active.
fn sync_title_bar(hwnd: HWND, st: &WinState) {
    if !st.o.custom_title_bar {
        return;
    }
    let Some(core) = st.core.borrow().clone() else { return };
    let maximized = unsafe { IsZoomed(hwnd).as_bool() };
    let js = format!("window.__xljsTitleBarState && window.__xljsTitleBarState({maximized}, {})", st.active.get());
    unsafe {
        let _ = core.ExecuteScript(&hstr(&js), None);
    }
}

/// The client area of a window with a custom title bar: the whole window
/// but the side and bottom borders (which stay for resizing), and while
/// maximized the part that is off the screen.
unsafe fn nc_calc_size(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let p = lparam.0 as *mut NCCALCSIZE_PARAMS;
        let before = (*p).rgrc[0];
        let r = DefWindowProcW(hwnd, WM_NCCALCSIZE, wparam, lparam);
        if r.0 != 0 {
            return r;
        }
        let border = before.bottom - (*p).rgrc[0].bottom;
        (*p).rgrc[0].top = before.top + if IsZoomed(hwnd).as_bool() { border.max(0) } else { 0 };
        LRESULT(0)
    }
}

fn resize(hwnd: HWND, c: &ICoreWebView2Controller) {
    unsafe {
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let _ = c.SetBounds(r);
    }
}

fn focus(hwnd: HWND, c: &ICoreWebView2Controller) {
    unsafe {
        if !IsIconic(hwnd).as_bool() {
            let _ = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(st) = state(hwnd) {
        let controller = st.controller.borrow().clone();
        match msg {
            WM_NCCALCSIZE if st.o.custom_title_bar && wparam.0 != 0 => return unsafe { nc_calc_size(hwnd, wparam, lparam) },
            WM_SIZE => {
                if let Some(c) = &controller
                    && wparam.0 as u32 != SIZE_MINIMIZED
                {
                    resize(hwnd, c);
                    sync_title_bar(hwnd, &st);
                }
            }
            WM_MOVE => {
                if let Some(c) = &controller {
                    unsafe {
                        let _ = c.NotifyParentWindowPositionChanged();
                    }
                }
            }
            WM_ACTIVATE => {
                st.active.set(wparam.0 & 0xFFFF != 0);
                sync_title_bar(hwnd, &st);
                if let Some(c) = &controller
                    && wparam.0 & 0xFFFF != 0
                    && (wparam.0 >> 16) & 0xFFFF == 0
                {
                    focus(hwnd, c);
                }
            }
            WM_DESTROY => destroyed(hwnd, &st),
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn destroyed(hwnd: HWND, st: &Rc<WinState>) {
    if st.closed.replace(true) {
        return;
    }
    // Close re-enters wnd_proc (WebView2 destroys its child windows): no
    // borrow may be held across it.
    let controller = st.controller.borrow_mut().take();
    if let Some(c) = controller {
        unsafe {
            let _ = c.Close();
        }
    }
    st.core.borrow_mut().take();
    st.env.borrow_mut().take();
    STATES.with(|s| s.borrow_mut().remove(&(hwnd.0 as isize)));
    for icon in st.icons {
        if !icon.is_invalid() {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
    WINDOWS.with(|w| w.borrow_mut().retain(|x| *x != hwnd));
    if let Some(f) = &st.o.on_closed {
        f();
    }
}

#[allow(dead_code)]
fn unused() -> (PCWSTR, String) {
    (PCWSTR::null(), i18n::lang())
}
