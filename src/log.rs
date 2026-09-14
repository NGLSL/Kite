//! 简单日志：同时写 stderr 与应用数据目录 kite.log（Windows 下 eprintln 可能看不到）。
//!
//! 日志由多个启动阶段进程共同写入（提权引导进程会很快创建子进程），
//! 因此这里用一个短暂的旁车锁串行化追加和裁剪。文件达到上限后保留最新
//! 内容，旧内容直接覆盖，避免异常启动循环把用户磁盘写满。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 单个日志文件的硬上限（5,000,000 bytes）。达到上限后只保留最新记录。
pub const MAX_LOG_BYTES: u64 = 5_000_000;
const LOCK_WAIT: Duration = Duration::from_millis(500);
const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);

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
            let _ = append_line(p, line.as_bytes());
        }
    }
}

pub fn default_path_under(dir: &Path) -> PathBuf {
    dir.join("kite.log")
}

fn append_line(path: &Path, line: &[u8]) -> std::io::Result<()> {
    // Bootstrap processes are separate OS processes, so a Rust Mutex alone is
    // insufficient. If the sidecar cannot be acquired, drop this log record
    // rather than writing without coordination and exceeding the size cap.
    // The caller ignores log errors so a filesystem filter never blocks boot.
    let lock_path = lock_path(path);
    let lock = acquire_lock(&lock_path).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::WouldBlock, "kite log lock unavailable")
    })?;
    let result = append_line_locked(path, line, MAX_LOG_BYTES);
    // Windows may reject deleting an open file depending on the sharing mode
    // used by a filesystem filter; release the handle first.
    drop(lock);
    let _ = fs::remove_file(lock_path);
    result
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".lock");
    PathBuf::from(name)
}

fn acquire_lock(path: &Path) -> Option<File> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => return Some(file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A crashed bootstrap must not leave logging blocked forever.
                let stale = fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > LOCK_STALE_AFTER);
                if stale {
                    let _ = fs::remove_file(path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return None,
        }
    }
}

fn append_line_locked(path: &Path, line: &[u8], max_bytes: u64) -> std::io::Result<()> {
    let max = usize::try_from(max_bytes).unwrap_or(usize::MAX);
    if max == 0 {
        return Ok(());
    }

    let current_len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if current_len.saturating_add(line.len() as u64) <= max_bytes {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line)?;
        return file.flush();
    }

    // Keep the newest bytes that can fit beside the incoming record. Start at
    // a line boundary when possible so the next log remains readable.
    if line.len() >= max {
        let start = line.len() - max;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(&line[start..])?;
        return file.flush();
    }

    let retain = max - line.len();
    let mut tail = Vec::new();
    if current_len > 0 {
        let mut old = File::open(path)?;
        let start = current_len.saturating_sub(retain as u64);
        old.seek(SeekFrom::Start(start))?;
        old.read_to_end(&mut tail)?;
        if start > 0 {
            if let Some(pos) = tail.iter().position(|byte| *byte == b'\n') {
                tail.drain(..=pos);
            } else {
                tail.clear();
            }
        }
    }

    if tail.len() > retain {
        let start = tail.len() - retain;
        tail.drain(..start);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(&tail)?;
    file.write_all(line)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "kite-log-{name}-{}-{}",
            std::process::id(),
            nonce
        ))
    }

    #[test]
    fn rolling_log_keeps_latest_content_under_limit() {
        let path = temp_log("roll");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(lock_path(&path));
        for i in 0..40 {
            let line = format!("record-{i:02}-{}\n", "x".repeat(8));
            append_line_locked(&path, line.as_bytes(), 128).expect("append test log");
        }
        let bytes = fs::read(&path).expect("read test log");
        assert!(bytes.len() <= 128);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("record-39"), "latest record must survive rotation");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn normal_log_append_does_not_rotate_early() {
        let path = temp_log("append");
        let _ = fs::remove_file(&path);
        append_line_locked(&path, b"first\n", 128).expect("append test log");
        append_line_locked(&path, b"second\n", 128).expect("append test log");
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn append_releases_sidecar_lock() {
        let path = temp_log("lock");
        let lock = lock_path(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&lock);
        append_line(&path, b"locked append\n").expect("append test log");
        assert!(!lock.exists(), "sidecar lock must not remain after append");
        let _ = fs::remove_file(&path);
    }
}
