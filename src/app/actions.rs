//! 结果上下文动作：打开所在文件夹。
//! 目标一律来自索引（AppItem / Everything 结果 id），绝不把用户输入拼进 shell。

use std::path::Path;

fn explorer_select_args(target: &Path) -> Vec<std::ffi::OsString> {
    vec!["/select,".into(), target.as_os_str().to_owned()]
}

/// 非文件系统目标前缀（UWP / 系统设置 URI / 内置动作）。
const NON_FS_PREFIXES: &[&str] = &[
    "shell:",
    "ms-settings:",
    "ms-clock:",
    "ms-contact-support:",
    "kite:",
];

/// 结果是否指向真实文件系统路径（决定「打开所在文件夹」是否可用）。
/// 浏览器 / 网页搜索项的 target 是 URL，同样不算。
pub fn has_fs_target(target: &str) -> bool {
    let t = target.trim();
    if t.is_empty() || NON_FS_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return false;
    }
    !(t.starts_with("http://") || t.starts_with("https://"))
}

/// 在资源管理器中定位文件；目录则直接打开该目录。
/// 原生实现：ShellExecute 打开目录，explorer /select 定位文件。
pub fn open_containing_folder(target: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let t = target.trim();
    if !has_fs_target(t) {
        return Err("该结果没有所在文件夹".into());
    }
    let path = Path::new(t);
    if !path.exists() {
        return Err(format!("目标不存在: {t}"));
    }
    if path.is_dir() {
        // 打开目录：ShellExecute 同源能力（与启动逻辑一致）
        super::uwp::launch_shell_path(t)
    } else {
        Command::new("explorer.exe")
            .args(explorer_select_args(path))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开所在文件夹失败: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_targets_recognized() {
        assert!(has_fs_target(r"C:\Program Files\App\app.exe"));
        assert!(has_fs_target(r"C:\Users\me\Documents"));
        assert!(has_fs_target(r"c:\lower\case\path.dll"));
    }

    #[test]
    fn non_fs_targets_rejected() {
        assert!(!has_fs_target("shell:AppsFolder\\App!App"));
        assert!(!has_fs_target("ms-settings:display"));
        assert!(!has_fs_target("kite:settings"));
        assert!(!has_fs_target("https://example.com/search?q=x"));
        assert!(!has_fs_target("http://example.com"));
        assert!(!has_fs_target(""));
        assert!(!has_fs_target("   "));
    }

    #[test]
    fn explorer_select_switch_and_path_are_separate_arguments() {
        let path = Path::new(r"C:\Program Files\Kite\kite.exe");
        let args = explorer_select_args(path);

        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "/select,");
        assert_eq!(args[1], path.as_os_str());
    }
}
