//! 提取并缓存 Windows 应用图标。任何失败都不得阻塞搜索。

mod extract;

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 将图标 PNG 缓存到 `icon_dir`，返回绝对路径。
/// `icon_src` 为首选源（lnk 的 icon_location 或 exe 路径）；失败时再试 target。
pub fn cache_icon(icon_dir: &Path, id: &str, icon_src: Option<&str>) -> Option<String> {
    let candidates = collect_candidates(icon_src);
    if candidates.is_empty() {
        return None;
    }
    std::fs::create_dir_all(icon_dir).ok()?;
    let out = icon_dir.join(format!("{}.png", hash_file_name(id)));
    if out.exists() {
        return Some(out.to_string_lossy().to_string());
    }
    for src in &candidates {
        let src_path = expand_env_path(src);
        if !src_path.exists() {
            continue;
        }
        if let Some(png) = extract::extract_icon_from_file(&src_path) {
            if std::fs::write(&out, &png).is_ok() {
                return Some(out.to_string_lossy().to_string());
            }
        }
    }
    crate::log::info(&format!(
        "icon fail id={id} tried={candidates:?}"
    ));
    None
}

/// 图标源候选：首选传入路径；exe 可再试自身。
fn collect_candidates(icon_src: Option<&str>) -> Vec<String> {
    let mut list = Vec::new();
    if let Some(s) = icon_src {
        let s = s.trim().trim_matches('"');
        if !s.is_empty() {
            list.push(s.to_string());
        }
    }
    list
}

/// 展开 `%windir%` 等环境变量；失败则原样返回。
fn expand_env_path(raw: &str) -> PathBuf {
    if !raw.contains('%') {
        return PathBuf::from(raw);
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(raw)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
        let need = ExpandEnvironmentStringsW(windows::core::PCWSTR(wide.as_ptr()), None);
        if need == 0 {
            return PathBuf::from(raw);
        }
        let mut buf = vec![0u16; need as usize];
        let n = ExpandEnvironmentStringsW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut buf));
        if n == 0 || n > need {
            return PathBuf::from(raw);
        }
        let s = String::from_utf16_lossy(&buf[..n as usize - 1]);
        PathBuf::from(s)
    }
}

fn hash_file_name(id: &str) -> String {
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_windir() {
        let p = expand_env_path(r"%windir%\System32\notepad.exe");
        let s = p.to_string_lossy();
        assert!(!s.contains('%'), "still has env token: {s}");
        assert!(s.to_lowercase().contains("notepad.exe"), "{s}");
    }

    #[test]
    fn expand_plain_unchanged() {
        let p = expand_env_path(r"C:\Windows\notepad.exe");
        assert_eq!(p.to_string_lossy(), r"C:\Windows\notepad.exe");
    }
}
