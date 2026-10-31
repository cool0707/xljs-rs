//! The xljs command: global options, subcommands (pack, check, init, vba,
//! version, help), the start screen, and running scripts.

mod console;
mod launch;
mod vba;

use std::path::Path;
use std::sync::Arc;

use crate::engine::{self, Options};
use crate::meta::{AssetFs, Meta};
use crate::ui::{self, Ui};
use crate::{Error, Result, err, i18n, pack, t};

pub use console::ConsoleState;

/// The version (set by the build: XLJS_VERSION, else the crate version).
pub fn version() -> String {
    option_env!("XLJS_VERSION").map(str::to_string).unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")))
}

fn usage() -> String {
    t!("cli.usage", i18n::available().join("|"))
}

/// Global options parsed from the command line.
#[derive(Default)]
struct Flags {
    ui: Option<String>,
    answers: Option<String>,
    lang: Option<String>,
    debug: Option<String>,
    help: bool,
    version: bool,
}

/// Parses the global options like Go's flag package (-x, --x, -x=v,
/// --x v); the first other argument and everything after it is returned.
fn parse_flags(args: &[String]) -> Result<(Flags, Vec<String>)> {
    let mut f = Flags::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            break;
        }
        if !a.starts_with('-') || a == "-" {
            break;
        }
        let body = a.trim_start_matches('-');
        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (body, None),
        };
        let take = |i: &mut usize| -> Result<String> {
            if let Some(v) = &value {
                return Ok(v.clone());
            }
            *i += 1;
            args.get(*i).cloned().ok_or_else(|| Error::msg(format!("flag needs an argument: -{name}")))
        };
        match name {
            "ui" => f.ui = Some(take(&mut i)?),
            "ui-answers" => f.answers = Some(take(&mut i)?),
            "lang" => f.lang = Some(take(&mut i)?),
            "debug" => f.debug = Some(value.clone().unwrap_or_else(|| "script".into())),
            "help" | "h" => f.help = true,
            "version" | "v" => f.version = true,
            _ => return Err(Error::msg(format!("flag provided but not defined: -{name}"))),
        }
        i += 1;
    }
    Ok((f, args[i..].to_vec()))
}

/// Runs xljs with the arguments (without the program name) and returns the
/// exit code.
pub fn main(args: Vec<String>) -> i32 {
    let con = console::setup();
    let code = run(&con, args);
    con.finish();
    code
}

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}

fn run(con: &ConsoleState, args: Vec<String>) -> i32 {
    install_interrupt();

    // A packed exe runs its embedded script; all arguments go to the script.
    if let Ok(Some(payload)) = pack::read_self() {
        let u = match choose_ui(&env("XLJS_UI"), &env("XLJS_UI_ANSWERS")) {
            Ok(u) => u,
            Err(e) => return con.fail(None, &e),
        };
        let exe = std::env::current_exe().unwrap_or_default();
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        let script = dir.join(&payload.name).to_string_lossy().into_owned();
        let m = payload.meta.clone().unwrap_or_default();
        apply_lang(&m, false);
        let assets: Arc<dyn AssetFs> = match payload.assets {
            Some(a) => Arc::from(a),
            None => Arc::new(m.asset_fs(&dir)),
        };
        let mut o = Options { script_path: script, code: Some(payload.code), args, meta: Some(m), assets: Some(assets), ..Default::default() };
        use_script_dir(con, &mut o);
        if let Err(e) = set_debug(&mut o, &env("XLJS_DEBUG"), &*u) {
            return con.fail(None, &e);
        }
        return run_script(con, u, o);
    }

    let (flags, rest) = match parse_flags(&args) {
        Ok(x) => x,
        Err(e) => {
            eprint!("{}", usage());
            return con.fail(None, &e);
        }
    };
    let explicit_lang = flags.lang.is_some();
    if let Some(l) = &flags.lang
        && !i18n::set_language(l)
    {
        return con.fail(None, &err!("cli.unknownLang", l, i18n::available().join(", ")));
    }
    if flags.version {
        println!("xljs {}", version());
        return 0;
    }
    if flags.help {
        print!("{}", usage());
        return 0;
    }
    if let Some(first) = rest.first() {
        match first.as_str() {
            "version" => {
                println!("xljs {}", version());
                return 0;
            }
            "help" | "/?" => {
                print!("{}", usage());
                return 0;
            }
            "pack" => return cmd_pack(con, &rest[1..]),
            "check" => return cmd_check(con, &rest[1..]),
            "init" => return cmd_init(con, &rest[1..]),
            "vba" => return vba::cmd_vba(con, &rest[1..]),
            _ => {}
        }
    }
    let ui_mode = flags.ui.unwrap_or_else(|| env("XLJS_UI"));
    let answers = flags.answers.unwrap_or_else(|| env("XLJS_UI_ANSWERS"));
    let u = match choose_ui(&ui_mode, &answers) {
        Ok(u) => u,
        Err(e) => return con.fail(None, &e),
    };
    let debug = flags.debug.unwrap_or_else(|| env("XLJS_DEBUG"));
    if rest.is_empty() {
        // Double-clicked without a script: ask what to do.
        return launch::launch(con, u, explicit_lang);
    }
    start(con, u, rest, explicit_lang, &debug)
}

/// Counts Ctrl+C: the first one only cancels while a progress indicator is
/// open; otherwise (or the second time) the process ends.
fn install_interrupt() {
    let _ = ctrlc::set_handler(|| {
        let n = ui::notify_interrupt();
        if !ui::progress_active() || n >= 2 {
            eprintln!("\n{}", t!("cli.interrupted"));
            std::process::exit(130);
        }
        eprintln!("\n{}", t!("cli.cancelRequested"));
    });
}

/// Runs the script rest[0] with the arguments rest[1..], or writes out the
/// VBA code when workbooks are given instead of a script.
fn start(con: &ConsoleState, u: Arc<dyn Ui>, rest: Vec<String>, explicit_lang: bool, debug: &str) -> i32 {
    if vba::is_workbook(&rest[0]) {
        return vba::drop_vba(con, u, &rest);
    }
    let m = match read_meta(&rest[0]) {
        Ok(m) => m,
        Err(e) => return con.fail(Some(&*u), &e),
    };
    apply_lang(&m, explicit_lang);
    let mut o = Options { script_path: rest[0].clone(), args: rest[1..].to_vec(), meta: Some(m), ..Default::default() };
    use_script_dir(con, &mut o);
    if let Err(e) = set_debug(&mut o, debug, &*u) {
        return con.fail(Some(&*u), &e);
    }
    run_script(con, u, o)
}

/// Makes the script's folder the working folder when xljs was started from
/// Explorer (double-clicked, or a file dropped onto the exe or the
/// script), so that relative paths mean "next to the script". Arguments
/// naming existing files are made absolute first.
fn use_script_dir(con: &ConsoleState, o: &mut Options) {
    if !con.from_explorer() {
        return;
    }
    for a in &mut o.args {
        if Path::new(a).exists()
            && let Ok(abs) = std::path::absolute(&a)
        {
            *a = abs.to_string_lossy().into_owned();
        }
    }
    let Ok(abs) = std::path::absolute(&o.script_path) else { return };
    o.script_path = abs.to_string_lossy().into_owned();
    if let Some(d) = abs.parent() {
        let _ = std::env::set_current_dir(d);
    }
}

/// Applies the --debug (XLJS_DEBUG) value: "script" (also true, 1, on)
/// runs the script in the debugger window, which needs the native UI;
/// "window" only enables the developer tools of the script's windows.
fn set_debug(o: &mut Options, value: &str, u: &dyn Ui) -> Result<()> {
    match value.trim().to_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => {}
        "1" | "true" | "yes" | "on" | "script" => {
            o.debug = true;
            if u.name() == "native" {
                o.debug_script = true;
            } else {
                eprintln!("{}", t!("cli.debugNeedsWindows"));
            }
        }
        "window" => o.debug = true,
        _ => return Err(err!("cli.unknownDebug", value)),
    }
    Ok(())
}

/// Parses the metadata block of a script and prints its warnings.
fn read_meta(script: &str) -> Result<Meta> {
    let (m, warnings) = crate::meta::parse_file(script)?;
    for w in warnings {
        eprintln!("{w}");
    }
    Ok(m)
}

/// Switches to the script's @lang unless the language was chosen
/// explicitly (--lang or XLJS_LANG).
fn apply_lang(m: &Meta, explicit: bool) {
    if m.lang.is_empty() || explicit || !env("XLJS_LANG").is_empty() {
        return;
    }
    if !i18n::set_language(&m.lang) {
        eprintln!("{}", t!("cli.unknownMetaLang", m.lang, i18n::available().join(", ")));
    }
}

fn choose_ui(mode: &str, answers: &str) -> Result<Arc<dyn Ui>> {
    match mode.to_lowercase().as_str() {
        "" | "auto" => {
            if ui::native_available() {
                ui::new_native()
            } else {
                Ok(Arc::new(ui::Console::new()))
            }
        }
        "native" => ui::new_native(),
        "console" | "cui" | "cli" => Ok(Arc::new(ui::Console::new())),
        "mock" => Ok(Arc::new(ui::Mock::new(answers, Some(ui::Mock::stderr_log()))?)),
        _ => Err(err!("cli.unknownUI", mode)),
    }
}

/// Runs the script on its own thread; this (main) thread serves the UI
/// meanwhile.
fn run_script(con: &ConsoleState, u: Arc<dyn Ui>, mut o: Options) -> i32 {
    o.version = version();
    let r = ui::run(u.clone(), move |su| {
        o.ui = Some(su);
        engine::run(o)
    });
    match r {
        Ok(code) => code,
        Err(e) => con.fail(Some(&*u), &e),
    }
}

fn cmd_check(con: &ConsoleState, args: &[String]) -> i32 {
    if args.len() != 1 {
        eprintln!("{}", t!("cli.usageCheck"));
        return 2;
    }
    if let Err(e) = read_meta(&args[0]) {
        return con.fail(None, &e);
    }
    if let Err(e) = engine::bundle(&args[0]) {
        return con.fail(None, &e);
    }
    println!("{}", t!("cli.checkOK"));
    0
}

/// Splits arguments into "-o value" and the rest (flags before or after).
fn split_out(args: &[String]) -> Result<(Option<String>, Vec<String>)> {
    let mut out = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-o" || a == "--o" {
            i += 1;
            out = Some(args.get(i).cloned().ok_or_else(|| Error::msg("flag needs an argument: -o"))?);
        } else if let Some(v) = a.strip_prefix("-o=").or_else(|| a.strip_prefix("--o=")) {
            out = Some(v.to_string());
        } else if a.starts_with('-') && a.len() > 1 {
            return Err(Error::msg(format!("flag provided but not defined: {a}")));
        } else {
            rest.push(a.clone());
        }
        i += 1;
    }
    Ok((out, rest))
}

fn cmd_pack(con: &ConsoleState, args: &[String]) -> i32 {
    let (out, rest) = match split_out(args) {
        Ok(x) => x,
        Err(e) => return con.fail(None, &e),
    };
    if rest.len() != 1 {
        eprintln!("{}", t!("cli.usagePack"));
        return 2;
    }
    let script = &rest[0];
    let out = out.unwrap_or_else(|| pack_output(script));
    if let Err(e) = pack_script(script, &out) {
        return con.fail(None, &e);
    }
    println!("{}", t!("cli.created", out));
    0
}

/// The default exe for the script: its name with .exe (no extension
/// outside Windows), next to it.
pub(crate) fn pack_output(script: &str) -> String {
    let p = Path::new(script);
    let base = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = if cfg!(windows) { ".exe" } else { "" };
    p.with_file_name(format!("{base}{ext}")).to_string_lossy().into_owned()
}

/// Writes out: this exe with the bundled script embedded.
pub(crate) fn pack_script(script: &str, out: &str) -> Result<()> {
    let m = read_meta(script)?;
    let code = engine::bundle(script)?;
    let dir = Path::new(script).parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = if dir.as_os_str().is_empty() { std::path::PathBuf::from(".") } else { dir };
    let name = Path::new(script).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let p = pack::Payload { name, code, assets: Some(Box::new(m.asset_fs(&dir))), meta: Some(m) };
    pack::write(Path::new(out), &p)
}

const JSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2023",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "moduleDetection": "force",
    "allowImportingTsExtensions": true,
    "noEmit": true,
    "checkJs": true,
    "strict": false,
    "lib": ["ES2023"]
  },
  "include": ["*.js", "*.ts", "**/*.js", "**/*.ts", "xljs.d.ts"]
}
"#;

fn cmd_init(con: &ConsoleState, args: &[String]) -> i32 {
    let dir = args.first().cloned().unwrap_or_else(|| ".".into());
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return con.fail(None, &Error::path("mkdir", dir, e));
    }
    let dts = Path::new(&dir).join("xljs.d.ts");
    if let Err(e) = std::fs::write(&dts, crate::DTS) {
        return con.fail(None, &Error::path("open", dts.display().to_string(), e));
    }
    println!("{}", t!("cli.created", dts.display()));
    let jsconfig = Path::new(&dir).join("jsconfig.json");
    if !jsconfig.exists() {
        if let Err(e) = std::fs::write(&jsconfig, JSCONFIG) {
            return con.fail(None, &Error::path("open", jsconfig.display().to_string(), e));
        }
        println!("{}", t!("cli.created", jsconfig.display()));
    }
    0
}

#[cfg(test)]
mod tests;
