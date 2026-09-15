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
