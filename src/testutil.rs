//! Helpers for the tests: the process-wide state (language, working
//! folder, environment) is shared, so tests that touch it take the lock.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::sync::atomic::{AtomicU64, Ordering};

static LOCK: Mutex<()> = Mutex::new(());

/// Holds the global test lock with the language set; the language goes
/// back to Japanese (what the script tests expect) when it is dropped.
pub struct LangGuard {
    _g: MutexGuard<'static, ()>,
}

impl Drop for LangGuard {
    fn drop(&mut self) {
        crate::i18n::set_language("ja");
    }
}

pub fn lock_lang(lang: &str) -> LangGuard {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    crate::i18n::set_language(lang);
    LangGuard { _g: g }
}

/// A new empty temporary folder.
pub fn tempdir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!("xljs-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// A path in the repository.
pub fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}
