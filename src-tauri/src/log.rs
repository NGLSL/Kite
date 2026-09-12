//! 简单日志：同时写 stderr 与应用数据目录 kite.log（Windows 下 eprintln 可能看不到）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn init(path: PathBuf) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut g) = LOG_PATH.lock() {
        *g = Some(path);
    }
}

pub fn info(msg: &str) {
    let line = format!("[kite] {msg}\n");
    eprint!("{line}");
    if let Ok(g) = LOG_PATH.lock() {
        if let Some(p) = g.as_ref() {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
            {
                let _ = f.write_all(line.as_bytes());
                let _ = f.flush();
            }
        }
    }
}

pub fn path() -> Option<PathBuf> {
    LOG_PATH.lock().ok().and_then(|g| g.clone())
}

pub fn default_path_under(dir: &Path) -> PathBuf {
    dir.join("kite.log")
}
