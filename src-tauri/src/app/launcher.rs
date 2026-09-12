use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::AppItem;

/// 启动索引中的应用。禁止把用户输入拼进 shell 字符串。
pub fn launch(item: &AppItem) -> Result<(), String> {
    let target = &item.target;
    if target.is_empty() {
        return Err("empty target".into());
    }

    let path = Path::new(target);
    if !path.exists() {
        return Err(format!("target not found: {target}"));
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
