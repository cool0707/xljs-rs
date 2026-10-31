use std::io;

use crate::meta::{AssetFs, covered};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}

/// The folder under ASSET_ORIGIN that serves the built-in pages (forms and
/// tables), e.g. https://app.xljs/_xljs/form.html.
pub const BUILTIN_PREFIX: &str = "_xljs/";

/// The page runtime (the xljs object, data-bind and friends) that windows
/// run in every page of the script before the page's own scripts.
pub const PAGE_SCRIPT: &str = include_str!("web/xljs.js");

/// A built-in file (form.html, table.html, their scripts and styles, Font
/// Awesome Free, the debugger page) by its path under web/.
pub fn builtin_file(name: &str) -> Option<&'static [u8]> {
    embedded::WEB_FILES.iter().find(|(n, _)| *n == name).map(|(_, d)| *d)
}

/// The built-in files as an AssetFs (paths relative to web/).
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinAssets;

impl AssetFs for BuiltinAssets {
    fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        builtin_file(name).map(<[u8]>::to_vec).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_string()))
    }
    fn files(&self, root: &str) -> io::Result<Vec<String>> {
        let roots = [root.to_string()];
        Ok(embedded::WEB_FILES.iter().map(|(n, _)| n.to_string()).filter(|n| covered(&roots, n)).collect())
    }
}

/// The Day.js files (dayjs.min.js, locale/*.js, plugin/*.js).
pub(crate) fn dayjs_files() -> &'static [(&'static str, &'static [u8])] {
    embedded::DAYJS_FILES
}
