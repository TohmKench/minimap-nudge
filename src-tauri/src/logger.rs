//! Tiny append-only log so a real match can be checked afterwards.
//! Rotates to `minimap-nudge.old.log` past 512 KB.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 512 * 1024;
static LOG_FILE: OnceLock<Mutex<PathBuf>> = OnceLock::new();

pub fn init(dir: &Path) -> PathBuf {
    let _ = fs::create_dir_all(dir);
    let path = dir.join("minimap-nudge.log");
    let _ = LOG_FILE.set(Mutex::new(path.clone()));
    path
}

pub fn log(msg: &str) {
    let line = format!("{} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    let Some(lock) = LOG_FILE.get() else { return };
    let path = lock.lock().unwrap();
    if fs::metadata(&*path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = fs::rename(&*path, path.with_extension("old.log"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&*path) {
        let _ = f.write_all(line.as_bytes());
    }
}
