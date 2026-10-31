//! Finds the installed WebView2 runtime and creates environments with it,
//! without WebView2Loader.dll (the exe stays a single file): the runtime's
//! EmbeddedBrowserWebView.dll is located from the EdgeUpdate registry keys
//! and its CreateWebViewEnvironmentWithOptionsInternal is called, as the
//! loader itself does.

use std::path::{Path, PathBuf};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler, ICoreWebView2EnvironmentOptions,
};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::*;
use windows::core::{HRESULT, Interface, PCWSTR, s};

use super::hstr;

const INSTALL_KEY: &str = "Software\\Microsoft\\EdgeUpdate\\ClientState\\";
const CHANNELS: [&str; 4] = [
    "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}", // stable
    "{2CD8A007-E189-409D-A2C8-9AF4EF3C72AA}", // beta
    "{0D50BFEC-CD6A-4F9A-964C-C7416E3ACB10}", // dev
    "{65C35B14-6C1D-4122-AC46-7148CC9D6497}", // canary
];
const MINIMUM_VERSION: [u32; 4] = [86, 0, 616, 0];

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x64"
    }
}

fn reg_string(root: HKEY, sub: &str, name: &str) -> Option<String> {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(root, &hstr(sub), Some(0), KEY_READ | KEY_WOW64_32KEY, &mut key).is_err() {
            return None;
        }
        let mut size = 0u32;
        let name = hstr(name);
        let ok = RegQueryValueExW(key, &name, None, None, None, Some(&mut size)).is_ok();
        let mut out = None;
        if ok && size > 0 {
            let mut buf = vec![0u16; size as usize / 2 + 1];
            let mut size2 = size;
            if RegQueryValueExW(key, &name, None, None, Some(buf.as_mut_ptr() as *mut u8), Some(&mut size2)).is_ok() {
                let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                out = Some(String::from_utf16_lossy(&buf[..n]));
            }
        }
        let _ = RegCloseKey(key);
        out
    }
}

fn parse_version(s: &str) -> Option<[u32; 4]> {
    let parts: Vec<u32> = s.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    if parts.len() != 4 {
        return None;
    }
    Some([parts[0], parts[1], parts[2], parts[3]])
}

/// The client DLL of the installed runtime and its version.
pub(super) fn find_installed() -> Option<(PathBuf, String)> {
    for channel in CHANNELS {
        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let Some(folder) = reg_string(root, &format!("{INSTALL_KEY}{channel}"), "EBWebView") else { continue };
            if folder.is_empty() {
                continue;
            }
            let version = Path::new(&folder).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(v) = parse_version(&version) else { continue };
            if v < MINIMUM_VERSION {
                continue;
            }
            let dll = Path::new(&folder).join("EBWebView").join(arch()).join("EmbeddedBrowserWebView.dll");
            if dll.is_file() {
                return Some((dll, version));
            }
        }
    }
    None
}

type CreateFn = unsafe extern "system" fn(
    check_running_instance: i32,
    runtime_type: i32,
    user_data_folder: PCWSTR,
    options: *mut core::ffi::c_void,
    handler: *mut core::ffi::c_void,
) -> HRESULT;

/// Starts creating an environment; handler receives it.
pub(super) fn create_environment(
    user_data: &str,
    options: &ICoreWebView2EnvironmentOptions,
    handler: &ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler,
) -> Result<(), String> {
    let Some((dll, _)) = find_installed() else { return Err("no WebView2 runtime found".into()) };
    unsafe {
        let module: HMODULE = LoadLibraryW(&hstr(&dll.to_string_lossy())).map_err(|e| format!("Loading DLL failed: {e}"))?;
        let Some(f) = GetProcAddress(module, s!("CreateWebViewEnvironmentWithOptionsInternal")) else {
            return Err("Unable to find CreateWebViewEnvironmentWithOptionsInternal".into());
        };
        let create: CreateFn = std::mem::transmute(f);
        // Settings from the environment must not change where the runtime
        // and its data come from.
        for k in ["WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER", "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", "WEBVIEW2_USER_DATA_FOLDER"] {
            std::env::set_var(k, "");
        }
        std::env::set_var("WEBVIEW2_RELEASE_CHANNEL_PREFERENCE", "0");
        let data = hstr(user_data);
        let hr = create(1, 0, PCWSTR(data.as_ptr()), options.as_raw(), handler.as_raw());
        hr.ok().map_err(|e| e.to_string())
    }
}
