// The GUI build (xljs.exe) has no console window; xljsc.exe is the same
// program built for the console subsystem (feature "console").
#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]

fn main() {
    // The main thread is the UI thread: dialogs (COM, STA) and windows are
    // created there, while scripts run on their own thread (see ui::run).
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(xljs::cli::main(args));
}
