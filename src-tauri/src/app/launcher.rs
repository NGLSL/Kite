use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::AppItem;

use super::uwp;

/// 启动索引中的应用。禁止把用户输入拼进 shell 字符串。
pub fn launch(item: &AppItem) -> Result<(), String> {
    let target = &item.target;
    if target.is_empty() {
        return Err("empty target".into());
    }

    // Store / shell / 系统设置 URI
    if target.starts_with("shell:")
        || target.starts_with("ms-settings:")
        || target.starts_with("ms-clock:")
        || target.starts_with("ms-contact-support:")
    {
        return uwp::launch_shell_path(target);
    }

    // 内置动作由 commands 层处理；此处不认 kite:
    if target.starts_with("kite:") {
        return Err("builtin action should be handled by launch_app".into());
    }

    // 文件/文件夹（Everything 结果）
    let path = Path::new(target);
    if path.is_dir() {
        return opener_open(target);
    }
    if !path.exists() {
        return Err(format!("target not found: {target}"));
    }

    // 文件用系统关联打开
    if !is_executable(path) {
        return opener_open(target);
    }

    let mut cmd = Command::new(target);
    if let Some(args) = &item.args {
        for a in args.split_whitespace() {
            cmd.arg(a);
        }
    }
    if let Some(dir) = &item.working_dir {
        let dir = Path::new(dir);
        if dir.is_dir() {
            cmd.current_dir(dir);
        }
    } else if let Some(parent) = path.parent() {
        cmd.current_dir(parent);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    Ok(())
}

fn is_executable(p: &Path) -> bool {
    p.extension()
        .map(|e| e.to_string_lossy().eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn opener_open(path: &str) -> Result<(), String> {
    // 使用 tauri-plugin-opener 会更稳；此处用 ShellExecute 同源能力
    super::uwp::launch_shell_path(path)
}
