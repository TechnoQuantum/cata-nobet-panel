//! Append-only diagnostic log at %LOCALAPPDATA%\NobetPanel\nobet_panel.log.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

pub fn path() -> PathBuf {
    crate::store::data_dir().join("nobet_panel.log")
}

pub fn write(level: &str, text: &str) {
    let _g = LOCK.lock();
    let p = path();
    if fs::metadata(&p).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) {
        let _ = fs::rename(&p, p.with_extension("log.1"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} [{level}] {text}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
    }
}
