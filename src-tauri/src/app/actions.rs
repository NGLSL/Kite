//! 结果上下文动作：打开所在文件夹。
//! 目标一律来自索引（AppItem / Everything 结果 id），绝不把用户输入拼进 shell。

use std::path::Path;

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
pub fn open_containing_folder(target: &str) -> Result<(), String> {
    let t = target.trim();
    if !has_fs_target(t) {
        return Err("该结果没有所在文件夹".into());
    }
    let path = Path::new(t);
    if !path.exists() {
        return Err(format!("目标不存在: {t}"));
    }
    if path.is_dir() {
        tauri_plugin_opener::open_path(path, None::<&str>)
            .map_err(|e| format!("打开文件夹失败: {e}"))
    } else {
        tauri_plugin_opener::reveal_item_in_dir(path)
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
}
