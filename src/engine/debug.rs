//! The debugger (xljs --debug).

use crate::api::Env;
use crate::{Result, err};

use super::Options;

/// The URL of the debugger window.
pub const DEBUG_PAGE: &str = "https://app.xljs/_xljs/debug.html";

pub(super) fn run(_o: &Options, _env: Env, _code: Vec<u8>) -> Result<i32> {
    Err(err!("engine.internalError", "debugger"))
}
