//! The high-RAM builder is short-lived so its allocator cannot become UI residency.

use kite_file_index::{CancelToken, Generation, Progress};
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

const BUILD_ARG: &str = "--kite-file-index-build";

pub(super) fn dispatch_entry() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|arg| arg != BUILD_ARG) {
        return false;
    }
    if args.len() != 3 {
        std::process::exit(2);
    }
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let lifetime = cancel.clone();
    std::thread::spawn(move || {
        // The parent keeps stdin open. EOF cancels an orphaned builder without
        // a service, scheduled task, or a second lifetime protocol.
        let mut byte = [0u8; 1];
        let _ = std::io::stdin().read(&mut byte);
        lifetime.store(true, Ordering::Release);
    });
    let progress = |event: Value| {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        let _ = serde_json::to_writer(&mut out, &event);
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    };
    let result =
        kite_file_index::build(Path::new(&args[1]), Path::new(&args[2]), cancel, &progress);
    match result {
        Ok(_) => {
            progress(serde_json::json!({"stage":"builder-complete"}));
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

pub(super) fn run(
    capture: &Path,
    destination: &Path,
    cancel: CancelToken,
    progress: Progress<'_>,
) -> Result<Generation, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .arg(BUILD_ARG)
        .arg(capture)
        .arg(destination)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW; no extra application UI.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动索引构建：{error}"))?;
    let _parent_lifetime = child.stdin.take();
    let stdout = child.stdout.take().ok_or("构建进度管道不可用")?;
    let stderr = child.stderr.take().ok_or("构建错误管道不可用")?;
    let (sender, receiver) = mpsc::sync_channel(16);
    let output = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let event = line.map_err(|error| error.to_string()).and_then(|line| {
                serde_json::from_str::<Value>(&line).map_err(|error| error.to_string())
            });
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut input = std::io::BufReader::new(stderr);
        let mut kept = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(count) = input.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let remaining = (16 * 1024usize).saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..count.min(remaining)]);
        }
        String::from_utf8_lossy(&kept).into_owned()
    });
    let mut stream_error = None;
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => progress(event),
            Ok(Err(error)) => {
                stream_error = Some(error);
                let _ = child.kill();
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Drop the receiver first to release a reader waiting on a full queue.
    drop(receiver);
    let status = child.wait().map_err(|error| error.to_string())?;
    let _ = output.join();
    let detail = errors.join().unwrap_or_default();
    if cancel.load(Ordering::Acquire) {
        return Err("索引构建已取消".into());
    }
    if let Some(error) = stream_error {
        return Err(format!("构建进度读取失败：{error}"));
    }
    if !status.success() {
        return Err(format!("索引构建失败：{}", detail.trim()));
    }
    Generation::load(destination).map_err(|error| error.to_string())
}
