//! 扫描各数据源共用的纯函数。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use sha2::{Digest, Sha256};

pub fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn hash_id(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\0");
    }
    h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 稳定启动身份 id：只依赖规范化 target + 原始 args，与展示来源无关。
/// 同一程序在桌面/开始菜单/App Paths 间迁移时 Pin、历史、Alias 可继续命中。
pub fn stable_item_id(target: &str, args: Option<&str>) -> String {
    hash_id(&[&normalize_path_key(target), args.unwrap_or("")])
}

/// 展示归并用启动身份：同一安装、同一动作（规范 exe 路径或 AUMID + args）。
/// `shell:AppsFolder\<绝对 exe 路径>` 与直接 exe 路径视为同一启动目标。
pub fn launch_identity(target: &str, args: Option<&str>) -> String {
    let body = strip_shell_appsfolder(target.trim());
    format!(
        "{}\u{1f}{}",
        normalize_path_key(body),
        args.unwrap_or("").trim()
    )
}

fn strip_shell_appsfolder(target: &str) -> &str {
    const MARKER: &str = "shell:appsfolder";
    if target.len() >= MARKER.len()
        && target.is_char_boundary(MARKER.len())
        && target[..MARKER.len()].eq_ignore_ascii_case(MARKER)
    {
        let rest = &target[MARKER.len()..];
        return rest.strip_prefix('\\').unwrap_or(rest);
    }
    target
}

/// 安装根目录：从 exe 父目录向上爬过版本号与通用子目录（bin/office6 等）。
pub fn install_root_dir(target: &str) -> Option<String> {
    let body = strip_shell_appsfolder(target.trim());
    if !body.contains('\\') && !body.contains('/') {
        return None;
    }
    let path = Path::new(body);
    let mut dir = path.parent()?;
    loop {
        let name = dir.file_name()?.to_string_lossy().to_string();
        if is_version_dir_name(&name) || is_generic_install_subdir(&name) {
            match dir.parent() {
                Some(parent) if parent != dir => dir = parent,
                _ => break,
            }
            continue;
        }
        break;
    }
    Some(normalize_path_key(&dir.to_string_lossy()))
}

fn is_version_dir_name(name: &str) -> bool {
    let mut dots = 0;
    let mut digits = 0;
    for c in name.chars() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c == '.' {
            dots += 1;
        } else {
            return false;
        }
    }
    digits >= 1 && dots >= 1
}

fn is_generic_install_subdir(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "bin"
            | "bin32"
            | "bin64"
            | "x86"
            | "x64"
            | "win32"
            | "win64"
            | "program"
            | "programs"
            | "app"
            | "apps"
            | "office6"
            | "office5"
            | "utility"
            | "uninstall"
            | "uninst"
    ) || (n.len() >= 2
        && n.starts_with("office")
        && n[6..].chars().all(|c| c.is_ascii_digit()))
}

/// 展示名族：去掉版本后缀后的小写紧凑名。
pub fn display_family_key(name: &str) -> String {
    let mut s = name.trim().to_lowercase();
    if let Some(open) = s.rfind(['(', '[']) {
        let close = if s[open..].starts_with('(') { ')' } else { ']' };
        if s.ends_with(close) {
            s.truncate(open);
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 同安装族名称兼容：展示名族完全相同（去版本后缀）。
/// 短名前缀（wps↔wps office）走 exe 词干链接，不用名称前缀，避免 VS↔VS Code 误合并。
pub fn names_share_install_family(a: &str, b: &str) -> bool {
    let a = display_family_key(a);
    let b = display_family_key(b);
    !a.is_empty() && a == b
}

/// 旧版 id（含 source）：用于一次性迁移历史到 stable id。
pub fn legacy_item_id(target: &str, args: Option<&str>, source: &str) -> String {
    hash_id(&[&normalize_path_key(target), args.unwrap_or(""), source])
}

/// 常见扫描 source 枚举，覆盖旧 id 可能取值。
pub const KNOWN_SOURCES: &[&str] = &[
    "start-menu",
    "desktop",
    "app-paths",
    "scoop",
    "uwp",
    "builtin",
    "builtin-system",
    "win-settings",
    "test",
];

pub fn normalize_path_key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

pub fn app_display_name(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.to_string())
}

pub fn is_skippable_shortcut(name: &str) -> bool {
    let lower = name.to_lowercase();
    const SKIP: &[&str] = &[
        "uninstall",
        "unins000",
        "help",
        "readme",
        "license",
        "documentation",
        "website",
        "release notes",
        "setup",
    ];
    SKIP.iter().any(|s| lower.contains(s))
}

/// `/from=startmenu` 等启动来源标记：已验证不改变“打开应用”动作。
fn is_from_source_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.starts_with("/from=") || lower.starts_with("-from=") || lower.starts_with("--from=")
}

/// 去掉来源标记后的参数串；卸载/修复/配置等动作 token 会保留。
pub fn strip_launch_source_args(args: &str) -> String {
    args.split_whitespace()
        .filter(|t| !is_from_source_token(t))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 归并用参数等价：完全相同，或仅差已验证的 `/from=*` 来源标记。
/// 不忽略卸载、修复、调试等会改变启动动作的参数。
pub fn args_equivalent_for_merge(a: Option<&str>, b: Option<&str>) -> bool {
    let a = a.unwrap_or("").trim();
    let b = b.unwrap_or("").trim();
    if a == b {
        return true;
    }
    strip_launch_source_args(a) == strip_launch_source_args(b)
}

/// 用户开始菜单 Programs（FOLDERID_Programs）。
pub fn user_programs_dir() -> Option<std::path::PathBuf> {
    known_folder(windows::Win32::UI::Shell::FOLDERID_Programs)
}

/// 公共开始菜单 Programs（FOLDERID_CommonPrograms）。
pub fn common_programs_dir() -> Option<std::path::PathBuf> {
    known_folder(windows::Win32::UI::Shell::FOLDERID_CommonPrograms)
}

fn programs_to_start_menu(programs: std::path::PathBuf) -> std::path::PathBuf {
    programs
        .parent()
        .map(|start| start.to_path_buf())
        .unwrap_or(programs)
}

/// 用户 Start Menu 根（Programs 的父目录）。
pub fn user_start_menu_dir() -> Option<std::path::PathBuf> {
    user_programs_dir().map(programs_to_start_menu)
}

/// 公共 Start Menu 根。
pub fn common_start_menu_dir() -> Option<std::path::PathBuf> {
    common_programs_dir().map(programs_to_start_menu)
}

fn known_folder(id: windows::core::GUID) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};

    unsafe {
        let pwstr = SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None).ok()?;
        if pwstr.is_null() {
            return None;
        }
        let path = pwstr.to_string().ok().map(std::path::PathBuf::from);
        CoTaskMemFree(Some(pwstr.0.cast()));
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_identity_merges_shell_folder_exe_with_direct_exe() {
        let a = launch_identity(r"C:\Program Files\Tencent\WeChat\WeChat.exe", None);
        let b = launch_identity(
            r"shell:AppsFolder\C:\Program Files\Tencent\WeChat\WeChat.exe",
            None,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn launch_identity_separates_different_args() {
        let a = launch_identity(r"C:\a\app.exe", None);
        let b = launch_identity(r"C:\a\app.exe", Some("--dev"));
        assert_ne!(a, b);
    }

    #[test]
    fn launch_identity_separates_different_exe() {
        let a = launch_identity(r"C:\A\WeChat\WeChat.exe", None);
        let b = launch_identity(r"C:\B\Weixin\Weixin.exe", None);
        assert_ne!(a, b);
    }

    #[test]
    fn launch_identity_keeps_uwp_aumid_distinct() {
        let uwp = launch_identity(r"shell:AppsFolder\TencentWeChat_abc!App", None);
        let desk = launch_identity(r"C:\Program Files\Tencent\WeChat\WeChat.exe", None);
        assert_ne!(uwp, desk);
    }

    #[test]
    fn install_root_strips_version_dir() {
        let root = install_root_dir(r"D:\Program Files\WPS Office\12.1.0.28505\office6\wps.exe")
            .unwrap();
        assert!(root.ends_with("wps office"), "{root}");
        let root2 = install_root_dir(r"D:\Program Files\WPS Office\ksolaunch.exe").unwrap();
        assert_eq!(root, root2);
    }

    #[test]
    fn wps_short_name_compatible_with_wps_office() {
        assert!(names_share_install_family(
            "WPS Office",
            "WPS Office (12.1.0.28505)"
        ));
        assert!(!names_share_install_family("wps", "WPS Office"));
        assert!(!names_share_install_family("notepad", "notepad++"));
        assert!(!names_share_install_family(
            "Visual Studio",
            "Visual Studio Code"
        ));
    }

    #[test]
    fn args_equivalent_only_ignores_from_source_tokens() {
        assert!(args_equivalent_for_merge(
            Some("/prometheus /fromksolaunch /from=startmenu"),
            Some("/prometheus /fromksolaunch /from=desktop_shortcut")
        ));
        assert!(args_equivalent_for_merge(None, Some("")));
        assert!(args_equivalent_for_merge(Some("--open"), Some("--open")));
        assert!(!args_equivalent_for_merge(
            Some("--open"),
            Some("--open --settings")
        ));
        assert!(!args_equivalent_for_merge(Some("/uninstall"), Some("")));
        assert!(!args_equivalent_for_merge(Some("/repair"), Some("/from=startmenu")));
    }
}
