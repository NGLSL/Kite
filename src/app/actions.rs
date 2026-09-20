//! 结果上下文动作：打开所在文件夹。
//! 索引结果与已验证的用户路径都通过固定系统接口打开，不拼接 shell 命令。

use std::path::{Path, PathBuf};

/// 仅接受普通盘符绝对路径或 UNC 路径；返回去掉成对外引号后的路径。
/// 不对文件名中的合法符号做 shell 规则过滤，因为打开时不经过 shell 命令拼接。
pub fn direct_path_candidate(input: &str) -> Option<PathBuf> {
    let trimmed = input.trim();
    let value = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);
    if value.is_empty()
        || value.contains(['\0', '\r', '\n', '"'])
        || value.starts_with(r"\\?\")
        || value.starts_with(r"\\.\")
    {
        return None;
    }
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
        && !value[2..].contains(':');
    let unc = value.starts_with(r"\\")
        && value[2..].split('\\').take(2).all(|part| !part.is_empty())
        && value[2..].contains('\\');
    (drive || unc).then(|| PathBuf::from(value))
}

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

    #[test]
    fn direct_path_accepts_ordinary_windows_names_and_rejects_non_paths() {
        assert_eq!(
            direct_path_candidate(r#""C:\Work\A & B 100%.txt""#),
            Some(PathBuf::from(r"C:\Work\A & B 100%.txt"))
        );
        assert_eq!(
            direct_path_candidate(r"\\server\share\report"),
            Some(PathBuf::from(r"\\server\share\report"))
        );
        assert!(direct_path_candidate(r"C:relative.txt").is_none());
        assert!(direct_path_candidate(r"\\server\").is_none());
        assert!(direct_path_candidate(r"\\?\C:\Windows").is_none());
        assert!(direct_path_candidate("https://example.com").is_none());
    }
}
