use std::sync::Arc;

use super::{ConsoleState, pack_output, pack_script, start};
use crate::ui::{Choice, FileFilter, PickerOptions, Ui};
use crate::t;

// What the start screen offers, in its order.
const LAUNCH_RUN: i64 = 0;
const LAUNCH_DEBUG: i64 = 1;
const LAUNCH_PACK: i64 = 2;
const LAUNCH_VBA: i64 = 3;

// Icons of the choices (drawn by tools/make_launch_icons.py).
const ICON_RUN: &[u8] = include_bytes!("icons/run.ico");
const ICON_DEBUG: &[u8] = include_bytes!("icons/debug.ico");
const ICON_PACK: &[u8] = include_bytes!("icons/pack.ico");
const ICON_VBA: &[u8] = include_bytes!("icons/vba.ico");

/// The start screen of xljs started without a script (double-clicked): it
/// asks whether to run, debug or pack a script, or to extract the VBA code
/// of macro workbooks, and asks for the files.
pub(super) fn launch(con: &ConsoleState, u: Arc<dyn Ui>, explicit_lang: bool) -> i32 {
    let choice = |label: &str, note: &str, icon: &[u8]| Choice { label: t!(label), note: t!(note), icon: icon.to_vec() };
    let choices = [
        choice("cli.launchRun", "cli.launchRunNote", ICON_RUN),
        choice("cli.launchDebug", "cli.launchDebugNote", ICON_DEBUG),
        choice("cli.launchPack", "cli.launchPackNote", ICON_PACK),
        choice("cli.launchVBA", "cli.launchVBANote", ICON_VBA),
    ];
    let Some(c) = u.choose("xljs", &t!("cli.launchMessage"), &choices) else {
        eprint!("{}", super::usage());
        return 2;
    };
    match c {
        LAUNCH_RUN | LAUNCH_DEBUG => {
            let Some(script) = pick_script(&*u, &t!("cli.selectScript")) else { return 0 };
            let debug = if c == LAUNCH_DEBUG { "script" } else { "" };
            start(con, u, vec![script], explicit_lang, debug)
        }
        LAUNCH_PACK => {
            let Some(script) = pick_script(&*u, &t!("cli.selectScriptToPack")) else { return 0 };
            launch_pack_script(con, &*u, &script)
        }
        LAUNCH_VBA => {
            let books = u.open_files(&PickerOptions {
                title: t!("cli.selectMacroBooks"),
                multiple: true,
                filters: vec![FileFilter {
                    description: t!("cli.macroBooks"),
                    patterns: super::vba::MACRO_PATTERNS.iter().map(|s| s.to_string()).collect(),
                }],
                ..Default::default()
            });
            if books.is_empty() {
                return 0;
            }
            super::vba::drop_vba(con, u, &books)
        }
        _ => 0,
    }
}

/// Asks for a script; None when cancelled.
fn pick_script(u: &dyn Ui, title: &str) -> Option<String> {
    let paths = u.open_files(&PickerOptions {
        title: title.to_string(),
        filters: vec![FileFilter { description: t!("cli.scripts"), patterns: vec!["*.js".into(), "*.mjs".into(), "*.ts".into()] }],
        ..Default::default()
    });
    paths.into_iter().next()
}

/// Asks where to save the exe for the script, creates it and offers to
/// show it in its folder.
fn launch_pack_script(con: &ConsoleState, u: &dyn Ui, script: &str) -> i32 {
    let out = pack_output(script);
    let abs = std::path::absolute(&out).unwrap_or_else(|_| out.clone().into());
    let Some(out) = u.save_file(&PickerOptions {
        title: t!("cli.packSaveTitle"),
        start_in: abs.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        suggested_name: abs.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        filters: vec![FileFilter { description: t!("cli.programs"), patterns: vec!["*.exe".into()] }],
        ..Default::default()
    }) else {
        return 0;
    };
    if let Err(e) = pack_script(script, &out) {
        return con.fail(Some(u), &e);
    }
    if u.confirm("xljs", &t!("cli.packed", out))
        && let Err(e) = u.show_in_folder(&[out])
    {
        return con.fail(Some(u), &e);
    }
    0
}
