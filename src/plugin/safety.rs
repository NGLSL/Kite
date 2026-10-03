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

/// 仅允许普通盘符绝对路径或 UNC，拒绝 URI、设备路径和数据流。
/// 调用方把整个路径作为 ShellExecuteW 的 lpFile，参数为空，不拼接命令。
/// 因此 &、%、单引号等合法文件名字符按字面保留。
pub fn plugin_path_allowed(path: &str) -> bool {
    let p = path.trim();
    if p.contains(['|', '>', '<', '"', '*', '?']) {
        return false;
    }
    crate::app::actions::direct_path_candidate(p).is_some()
        && !p.strip_prefix(r"\\").is_some_and(|unc| unc.contains(':'))
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
    fn path_accepts_literal_names_and_rejects_schemes_devices_and_streams() {
        assert!(plugin_path_allowed(r"C:\Windows\System32\calc.exe"));
        assert!(plugin_path_allowed(r"\\server\share\file.txt"));
        assert!(!plugin_path_allowed(
            "shell:AppsFolder\\Microsoft.WindowsStore"
        ));
        assert!(!plugin_path_allowed("ms-settings:clipboard"));
        assert!(plugin_path_allowed(r"C:\R&D 100%\report';^`.txt"));
        assert!(plugin_path_allowed(r"\\server\share\R&D 100%.txt"));
        assert!(!plugin_path_allowed("relative\\path.exe"));
        assert!(!plugin_path_allowed(r"\\?\C:\Windows"));
        assert!(!plugin_path_allowed(r"\\.\C:\Windows"));
        assert!(!plugin_path_allowed(r"C:\file.txt:stream"));
        assert!(!plugin_path_allowed(r"\\server\share\file.txt:stream"));
        assert!(!plugin_path_allowed(r"\\server\"));
        assert!(!plugin_path_allowed(r"C:\foo | calc.exe"));
        assert!(!plugin_path_allowed("C:\\foo\nbar.txt"));
        assert!(!plugin_path_allowed(""));
    }
}
