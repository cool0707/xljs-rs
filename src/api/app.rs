use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rquickjs::function::Rest;
use rquickjs::{Function, Object, Value};

use super::N;
use super::js::{self, to_str};
use crate::meta::{AssetFs, clean_path};
use crate::ui::{ASSET_ORIGIN, BUILTIN_PREFIX, BuiltinAssets, WindowOptions, check_icon};
use crate::{Result, err};

/// The URL of an HTML file given relative to the script folder ("" for an
/// empty page) and its name in the assets.
pub fn page_url(html: &str) -> Result<(String, String)> {
    if html.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let p = clean_path(&html.replace('\\', "/"));
    if p.starts_with('/') || p == ".." || p.starts_with("../") || p.contains(':') {
        return Err(err!("app.badHTML", html));
    }
    Ok((format!("{ASSET_ORIGIN}{}", escape_path(&p)), p))
}

/// Escapes a URL path like Go's url.URL.EscapedPath.
fn escape_path(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')'
            | b'*' | b'+' | b',' | b';' | b'=' | b':' | b'@' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Serves the built-in pages under BUILTIN_PREFIX and the script's assets
/// everywhere else.
#[derive(Clone)]
pub struct WindowAssets {
    pub app: Option<Arc<dyn AssetFs>>,
}

impl AssetFs for WindowAssets {
    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        if let Some(rest) = name.strip_prefix(BUILTIN_PREFIX) {
            return BuiltinAssets.read(rest);
        }
        match &self.app {
            Some(a) => a.read(name),
            None => Err(io::Error::new(io::ErrorKind::NotFound, name.to_string())),
        }
    }
    fn files(&self, root: &str) -> io::Result<Vec<String>> {
        match &self.app {
            Some(a) => a.files(root),
            None => Ok(Vec::new()),
        }
    }
}

/// The message a page sends to ask a running task to stop. It is noted as
/// soon as it arrives, so that a script busy in a loop can see it
/// (cancelRequested) before the message itself is delivered.
const CANCEL_MESSAGE: &str = r#"{"type":"cancel"}"#;

pub(super) fn install(n: &N) {
    let env = &n.host.env;
    n.val("appMode", env.meta.is_app());
    n.val("appHTML", env.meta.html.clone());
    n.val("appWidth", env.meta.width as f64);
    n.val("appHeight", env.meta.height as f64);
    n.val("appTitleBar", env.meta.title_bar.clone());
    n.val("supportsWindows", env.ui.supports_windows());
    let assets: Arc<dyn AssetFs> = Arc::new(WindowAssets { app: env.assets.clone() });
    // The @icon file, read when the first window opens.
    let icon: std::rc::Rc<std::cell::RefCell<Option<Vec<u8>>>> = Default::default();
    // appOpen(html, title, width, height, onMessage(msg, files), fixedSize,
    //         customTitleBar, titleIcon) ->
    // {close(), closed: Promise, post(msg), isClosed(), cancelRequested(), clearCancel()}
    n.set("appOpen", move |h, a| {
        let html = a.str(0);
        let (url, name) = page_url(&html).map_err(|e| a.throw(&e))?;
        if !name.is_empty() && !assets.is_file(&name) {
            return Err(a.throw(&err!("app.htmlNotFound", html)));
        }
        if !h.env.meta.icon.is_empty() && icon.borrow().is_none() {
            let data = assets.read(&h.env.meta.icon).map_err(|_| a.throw(&err!("app.iconNotFound", h.env.meta.icon)))?;
            if !check_icon(&data) {
                return Err(a.throw(&err!("app.badIcon", h.env.meta.icon)));
            }
            *icon.borrow_mut() = Some(data);
        }
        let ctx = a.ctx.clone();
        let on_message = a.arg(4).as_function().cloned().map(|f| h.keep_fn(&ctx, f));
        let is_closed = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (closed, resolve, _) = ctx.promise()?;
        let resolve_id = h.keep_fn(&ctx, resolve);
        let poster = h.poster();
        // The open window keeps the event loop (and the process) alive.
        poster.add_pending();
        let (p1, c1) = (poster.clone(), cancel.clone());
        let (p2, ic) = (poster.clone(), is_closed.clone());
        let title = if js::nullish(&a.arg(1)) { h.env.title.clone() } else { to_str(&a.arg(1)) };
        let w = h.env.ui.open_window(WindowOptions {
            title,
            width: a.int(2),
            height: a.int(3),
            fixed_size: a.bool(5),
            custom_title_bar: a.bool(6),
            title_icon: if a.bool(6) { a.str(7) } else { String::new() },
            url,
            assets: Some(assets.clone()),
            debug: h.env.debug,
            icon: icon.borrow().clone().unwrap_or_default(),
            on_message: Some(Arc::new(move |msg: String, files: Vec<String>| {
                if msg == CANCEL_MESSAGE {
                    c1.store(true, Ordering::SeqCst);
                }
                let Some(id) = on_message else { return };
                p1.add_pending();
                p1.post(Box::new(move |ctx, host| {
                    if let Some(f) = host.kept_fn(ctx, id) {
                        f.call::<_, Value>((msg, files))?;
                    }
                    Ok(())
                }));
            })),
            on_closed: Some(Arc::new(move || {
                ic.store(true, Ordering::SeqCst);
                p2.post(Box::new(move |ctx, host| {
                    if let Some(f) = host.kept_fn(ctx, resolve_id) {
                        f.call::<_, Value>(())?;
                    }
                    host.drop_fn(resolve_id);
                    if let Some(id) = on_message {
                        host.drop_fn(id);
                    }
                    Ok(())
                }));
            })),
            ..Default::default()
        });
        let w = match w {
            Ok(w) => w,
            Err(e) => {
                poster.post(Box::new(|_, _| Ok(())));
                return Err(a.throw(&e));
            }
        };
        let wc = w.clone();
        h.on_cleanup(move || wc.close());
        let obj = Object::new(ctx.clone())?;
        let w1 = w.clone();
        obj.set("close", Function::new(ctx.clone(), move || w1.close())?)?;
        obj.set("closed", closed)?;
        let w2 = w.clone();
        obj.set(
            "post",
            Function::new(ctx.clone(), move |args: Rest<Value>| {
                let msg = args.0.first().filter(|v| !js::nullish(v)).map(to_str).unwrap_or_default();
                w2.post(msg);
            })?,
        )?;
        let ic = is_closed.clone();
        obj.set("isClosed", Function::new(ctx.clone(), move || ic.load(Ordering::SeqCst))?)?;
        let c2 = cancel.clone();
        obj.set("cancelRequested", Function::new(ctx.clone(), move || c2.load(Ordering::SeqCst))?)?;
        let c3 = cancel.clone();
        obj.set("clearCancel", Function::new(ctx.clone(), move || c3.store(false, Ordering::SeqCst))?)?;
        Ok(obj.into_value())
    });
}
