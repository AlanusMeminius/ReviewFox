//! Ephemeral handle to the main window's open Repository path (Phase B smoke).

use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static CURRENT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub fn set_current_repo_path(path: Option<PathBuf>) {
    CURRENT.with(|c| *c.borrow_mut() = path);
}

pub fn current_repo_path() -> Option<PathBuf> {
    CURRENT.with(|c| c.borrow().clone())
}
