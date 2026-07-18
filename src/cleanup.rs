//! Best-effort removal of test files when the user interrupts a run.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn register(path: PathBuf) {
    PENDING.lock().unwrap().push(path);
}

pub fn unregister(path: &Path) {
    PENDING.lock().unwrap().retain(|p| p != path);
}

/// Delete everything registered. Used both by the Ctrl+C handler and normal exits.
pub fn run() {
    let paths: Vec<PathBuf> = PENDING.lock().unwrap().drain(..).collect();
    for p in paths {
        let _ = std::fs::remove_file(&p);
    }
}

pub fn install_ctrlc_handler() {
    let _ = ctrlc::set_handler(|| {
        run();
        eprintln!("\ninterrupted — flashmark test files removed");
        std::process::exit(130);
    });
}
