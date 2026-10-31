use std::sync::Arc;

use rquickjs::function::Rest;
use rquickjs::{Function, Object, Value};

use super::js::{self, elems, get, has, nullish, to_bool, to_str};
use super::N;
use crate::ui::{FileFilter, FormOptions, PickerOptions, ProgressOptions, TableOptions};
use crate::{Error, i18n};

fn opt_str(v: &Value, def: &str) -> String {
    if nullish(v) { def.to_string() } else { to_str(v) }
}

pub(crate) fn picker_options(v: &Value) -> PickerOptions {
    let mut o = PickerOptions::default();
    if nullish(v) {
        return o;
    }
    o.title = opt_str(&get(v, "title"), "");
    o.multiple = has(v, "multiple") && to_bool(&get(v, "multiple"));
    o.start_in = opt_str(&get(v, "startIn"), "");
    o.suggested_name = opt_str(&get(v, "suggestedName"), "");
    for f in elems(&get(v, "filters")) {
        o.filters.push(FileFilter { description: opt_str(&get(&f, "description"), ""), patterns: js::strings(&get(&f, "patterns")) });
    }
    o
}

/// The folder for remembered settings (%APPDATA%\xljs), "" if unknown.
pub(crate) fn config_dir() -> String {
    if let Ok(d) = std::env::var("XLJS_CONFIG_DIR")
        && !d.is_empty()
    {
        return d;
    }
    let base = if cfg!(windows) {
        std::env::var("APPDATA").ok()
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME").ok().map(|h| format!("{h}/Library/Application Support"))
    } else {
        std::env::var("XDG_CONFIG_HOME").ok().filter(|s| !s.is_empty()).or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")))
    };
    match base {
        Some(b) if !b.is_empty() => std::path::Path::new(&b).join("xljs").to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

pub(super) fn install(n: &N) {
    n.set("uiAlert", |h, a| {
        h.env.ui.alert(&a.str(0), &a.str(1));
        a.undefined()
    });
    n.set("uiConfirm", |h, a| a.ret(h.env.ui.confirm(&a.str(0), &a.str(1))));
    n.set("uiPrompt", |h, a| match h.env.ui.prompt(&a.str(0), &a.str(1), &a.str(2)) {
        Some(s) => a.ret(s),
        None => a.null(),
    });
    n.set("uiOpen", |h, a| a.ret(h.env.ui.open_files(&picker_options(&a.arg(0)))));
    n.set("uiSave", |h, a| match h.env.ui.save_file(&picker_options(&a.arg(0))) {
        Some(p) => a.ret(p),
        None => a.null(),
    });
    n.set("uiDirectory", |h, a| match h.env.ui.pick_directory(&picker_options(&a.arg(0))) {
        Some(p) => a.ret(p),
        None => a.null(),
    });
    n.set("uiError", |h, a| {
        h.env.ui.show_error(&a.str(0), &a.str(1));
        a.undefined()
    });
    n.set("shellShowInFolder", |h, a| {
        h.env.ui.show_in_folder(&a.strings(0)).map_err(|e| a.throw(&e))?;
        a.undefined()
    });
    n.set("shellOpen", |h, a| {
        h.env.ui.open_path(&a.str(0)).map_err(|e| a.throw(&e))?;
        a.undefined()
    });
    // uiForm(optionsJSON) -> valuesJSON | null (UIs without windows)
    n.set("uiForm", |h, a| {
        let o: FormOptions = serde_json::from_str(&a.str(0)).map_err(|e| a.throw(&Error::from(e)))?;
        match h.env.ui.form(&o) {
            Some(v) => a.ret(serde_json::Value::Object(v).to_string()),
            None => a.null(),
        }
    });
    // uiTable(optionsJSON) -> selected indices | null (UIs without windows)
    n.set("uiTable", |h, a| {
        let o: TableOptions = serde_json::from_str(&a.str(0)).map_err(|e| a.throw(&Error::from(e)))?;
        match h.env.ui.table(&o) {
            Some(sel) => a.ret(sel.into_iter().map(|i| i as f64).collect::<Vec<f64>>()),
            None => a.null(),
        }
    });
    n.set("configDir", |_, a| a.ret(config_dir()));
    // displayText(s) translates library messages for display.
    n.set("displayText", |_, a| a.ret(i18n::text(&a.str(0))));
    // uiProgress(title, total, message, cancellable) -> {update, cancelled, close}
    n.set("uiProgress", |h, a| {
        let p = h.env.ui.progress(&ProgressOptions {
            title: a.str(0),
            total: a.int(1),
            message: a.str(2),
            cancellable: nullish(&a.arg(3)) || a.bool(3),
        });
        let pc = p.clone();
        h.on_cleanup(move || pc.close());
        let obj = Object::new(a.ctx.clone())?;
        let p1: Arc<dyn crate::ui::Progress> = p.clone();
        obj.set(
            "update",
            Function::new(a.ctx.clone(), move |args: Rest<Value>| {
                let get = |i: usize| args.0.get(i).cloned();
                let int = |i: usize| get(i).map(|v| js::to_int(&v)).unwrap_or(0);
                let msg = get(2).filter(|v| !nullish(v)).map(|v| to_str(&v)).unwrap_or_default();
                p1.update(int(0), int(1), &msg);
            })?,
        )?;
        let p2 = p.clone();
        obj.set("cancelled", Function::new(a.ctx.clone(), move || p2.cancelled())?)?;
        let p3 = p.clone();
        obj.set("close", Function::new(a.ctx.clone(), move || p3.close())?)?;
        Ok(obj.into_value())
    });
}
