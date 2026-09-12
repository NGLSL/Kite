//! 提取并缓存 Windows 应用图标。任何失败都不得阻塞搜索。

mod extract;

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 将图标 PNG 缓存到 `icon_dir`，返回绝对路径。
pub fn cache_icon(icon_dir: &Path, id: &str, icon_src: Option<&str>) -> Option<String> {
    let src = icon_src?;
    let src_path = PathBuf::from(src);
    if !src_path.exists() {
        return None;
    }
    std::fs::create_dir_all(icon_dir).ok()?;
    let out = icon_dir.join(format!("{}.png", hash_file_name(id)));
    if out.exists() {
        return Some(out.to_string_lossy().to_string());
    }
    let png = extract::extract_icon_from_file(&src_path)?;
    std::fs::write(&out, &png).ok()?;
    Some(out.to_string_lossy().to_string())
}

fn hash_file_name(id: &str) -> String {
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
}
