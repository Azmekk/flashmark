//! Interrupt handling and best-effort removal of test files.
//!
//! Ctrl+C sets an abort flag that the I/O loops poll between blocks; the work
//! then unwinds normally, closing handles before deleting files. Deleting from
//! the handler thread is not safe: a file being written on a slow device keeps
//! the delete (and the process) stuck behind kernel-blocked I/O.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static ABORT: AtomicBool = AtomicBool::new(false);
static PRESSES: AtomicU32 = AtomicU32::new(0);

pub fn register(path: PathBuf) {
    PENDING.lock().unwrap().push(path);
}

pub fn unregister(path: &Path) {
    PENDING.lock().unwrap().retain(|p| p != path);
}

/// Delete everything registered. Called on normal unwind paths.
pub fn run() {
    let paths: Vec<PathBuf> = PENDING.lock().unwrap().drain(..).collect();
    for p in paths {
        let _ = std::fs::remove_file(&p);
    }
}

pub fn aborted() -> bool {
    ABORT.load(Ordering::Relaxed)
}

/// Poll point for I/O loops: errors with ErrorKind::Interrupted after Ctrl+C.
pub fn check_abort() -> std::io::Result<()> {
    if aborted() {
        Err(Error::new(ErrorKind::Interrupted, "interrupted by Ctrl+C"))
    } else {
        Ok(())
    }
}

pub fn install_ctrlc_handler() {
    let _ = ctrlc::set_handler(|| {
        if PRESSES.fetch_add(1, Ordering::SeqCst) == 0 {
            ABORT.store(true, Ordering::SeqCst);
            eprintln!(
                "\ninterrupting — finishing the current block, then cleaning up (Ctrl+C again to force quit)"
            );
        } else {
            run();
            std::process::exit(130);
        }
    });
}
