//! Runs xljs scripts.

use std::fmt;

/// A JavaScript error that terminated the script.
#[derive(Debug, Clone)]
pub struct ScriptError {
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Syntax and resolution errors found while bundling.
#[derive(Debug, Clone)]
pub struct BundleError {
    pub message: String,
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
