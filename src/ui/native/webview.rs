//! HTML windows (WebView2). Pages under ASSET_ORIGIN are served from the
//! window's assets; the page runtime (web/xljs.js) is injected into every
//! page; navigation away from the app opens the default browser instead.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, OnceLock, mpsc};

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Shell::{ExtractIconExW, SHCreateMemStream};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCWSTR, PWSTR, w};

use super::{WINDOWS, hstr, loader};
use crate::ui::{ASSET_ORIGIN, PAGE_SCRIPT, Window, WindowOptions, icon_image};
use crate::{Error, Result, err, i18n};

/// Where users get the WebView2 runtime.
pub const WEBVIEW2_DOWNLOAD_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

const DEFAULT_WIDTH: i64 = 960;
const DEFAULT_HEIGHT: i64 = 640;

/// Host → page messages start with this, so that page scripts can tell them
/// from anything else they receive.
const HOST_PREFIX: &str = "\u{2}";

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
    if webview2_version().is_empty() {
        return Err(err!("ui.webview2Missing", WEBVIEW2_DOWNLOAD_URL));
    }
    register_class();
    let width = if o.width > 0 { o.width } else { DEFAULT_WIDTH };
    let height = if o.height > 0 { o.height } else { DEFAULT_HEIGHT };
    let style = if o.fixed_size { WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX) } else { WS_OVERLAPPEDWINDOW };
    unsafe {
        let mut dpi = GetDpiForSystem();
        if dpi == 0 {
            dpi = 96;
        }
        let mut r = RECT { left: 0, top: 0, right: (width * dpi as i64 / 96) as i32, bottom: (height * dpi as i64 / 96) as i32 };
        let _ = AdjustWindowRectEx(&mut r, style, false, Default::default());
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as *mut _), Default::default());
        let x = work.left + 0.max((work.right - work.left - ww) / 2);
        let y = work.top + 0.max((work.bottom - work.top - wh) / 2);
        let hwnd = CreateWindowExW(
            Default::default(),
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
        });
        STATES.with(|s| s.borrow_mut().insert(hwnd.0 as isize, st.clone()));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if let Err(e) = embed(hwnd, &st) {
            // The caller learns from the error, not from on_closed.
            *st.closed.borrow_mut() = true;
            STATES.with(|s| s.borrow_mut().remove(&(hwnd.0 as isize)));
            let _ = DestroyWindow(hwnd);
            return Err(e);
        }
        WINDOWS.with(|w| w.borrow_mut().push(hwnd));
        // Not cancelled: the window stays open.
        super::notify::watch(&if title.is_empty() { "xljs".to_string() } else { title }, &crate::t!("ui.waiting"));
        Ok(Arc::new(WebWindow { hwnd: hwnd.0 as isize }))
    }
}

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

        // Messages from the page, with the paths of attached files.
        let on_message = st.o.on_message.clone();
        let mut token = 0i64;
        core.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                let Some(args) = args else { return Ok(()) };
                let mut p = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut p).is_err() {
                    return Ok(());
                }
                let msg = take_pwstr(p);
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
            WM_SIZE => {
                if let Some(c) = &controller
                    && wparam.0 as u32 != SIZE_MINIMIZED
                {
                    resize(hwnd, c);
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
