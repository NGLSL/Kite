//! 插件来源外部动作的安全闸门（open_url / open_path / Host API）。
//!
//! AGENTS.md：启动不得把未校验的插件/用户输入拼进 shell。
//! 核心索引启动走 AppItem；插件路径/URL 必须先过本模块。

/// 仅允许 http/https，拒绝 file:、javascript:、shell: 等。
pub fn plugin_url_allowed(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.contains('\0') || u.contains('\n') || u.contains('\r') {
        return false;
    }
    let lower = u.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// 仅允许盘符绝对路径或 UNC；拒绝 shell:/ms-settings: 等 scheme 与 shell 元字符。
pub fn plugin_path_allowed(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() || p.contains('\0') || p.contains('\n') || p.contains('\r') {
        return false;
    }
    if p.contains(['&', '|', '>', '<', '^', ';', '"', '\'', '`', '%']) {
        return false;
    }
    let b = p.as_bytes();
    let drive_abs = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/')
        && !p[2..].contains(':');
    // 普通 UNC（\\server\share\...），拒绝 \\?\ 设备路径
    let unc = p.starts_with(r"\\")
        && !p.starts_with(r"\\?\")
        && !p.starts_with(r"\\.\")
        && p.len() > 2
        && p[2..].contains('\\');
    drive_abs || unc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_only_http_schemes() {
        assert!(plugin_url_allowed("https://example.com/a"));
        assert!(plugin_url_allowed("http://example.com"));
        assert!(!plugin_url_allowed("file:///C:/Windows"));
        assert!(!plugin_url_allowed("javascript:alert(1)"));
        assert!(!plugin_url_allowed("shell:AppsFolder\\x"));
        assert!(!plugin_url_allowed(""));
    }

    #[test]
    fn path_rejects_schemes_and_metachars() {
        assert!(plugin_path_allowed(r"C:\Windows\System32\calc.exe"));
        assert!(plugin_path_allowed(r"\\server\share\file.txt"));
        assert!(!plugin_path_allowed("shell:AppsFolder\\Microsoft.WindowsStore"));
        assert!(!plugin_path_allowed("ms-settings:clipboard"));
        assert!(!plugin_path_allowed(r"C:\foo & calc.exe"));
        assert!(!plugin_path_allowed("relative\\path.exe"));
        assert!(!plugin_path_allowed(r"\\?\C:\Windows"));
        assert!(!plugin_path_allowed(""));
    }
}
