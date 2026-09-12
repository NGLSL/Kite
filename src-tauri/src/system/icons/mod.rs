//! 提取并缓存 Windows 应用图标。任何失败都不得阻塞搜索。
//! 按源类型分流（参考 ZeroLaunch-rs）：图片文件直接读内容、ico 取 256 大图、
//! 其余走 shell 提取；纯图片处理逻辑放本文件，Win32 调用集中在 extract.rs。

mod extract;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use image::RgbaImage;
use sha2::{Digest, Sha256};

/// 将图标 PNG 缓存到 `icon_dir`，返回绝对路径。
/// `icon_src` 为首选源（lnk 的 icon_location / UWP logo / exe 路径）；失败时再试 target。
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
        if let Some(png) = extract_any(&src_path) {
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

/// 按源类型分流；UWP 的 PNG logo 若走 shell 提取只会拿到「PNG 文件类型」的通用图标。
fn extract_any(path: &Path) -> Option<Vec<u8>> {
    match extract::classify(path) {
        extract::SourceKind::Image => decode_image_file(path),
        extract::SourceKind::Ico => {
            extract::extract_ico_large(path).or_else(|| extract::extract_shell_icon(path))
        }
        extract::SourceKind::Shell => extract::extract_shell_icon(path),
    }
}

/// 直接解码图片文件内容为图标（UWP logo 的主路径）。
fn decode_image_file(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    encode_png(&crop_and_fill(&img))
}

/// 文件/文件夹类型图标（扩展名 → 系统关联图标），进程内 + 磁盘双缓存。
/// 文件搜索结果量大且同类型重复，按扩展名缓存避免热路径反复提取。
pub fn cache_type_icon(icon_dir: &Path, file_name: &str, is_dir: bool) -> Option<String> {
    let key = if is_dir {
        "dir".to_string()
    } else {
        Path::new(file_name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())?
    };
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(g) = cache.lock() {
        if let Some(v) = g.get(&key) {
            return v.clone();
        }
    }
    let found = cache_type_icon_uncached(icon_dir, &key, is_dir);
    if let Ok(mut g) = cache.lock() {
        g.insert(key, found.clone());
    }
    found
}

fn cache_type_icon_uncached(icon_dir: &Path, key: &str, is_dir: bool) -> Option<String> {
    std::fs::create_dir_all(icon_dir).ok()?;
    let out = icon_dir.join(format!("{}.png", hash_file_name(&format!("filetype:{key}"))));
    if out.exists() {
        return Some(out.to_string_lossy().to_string());
    }
    let png = extract::extract_ext_icon(key, is_dir)?;
    std::fs::write(&out, &png).ok()?;
    Some(out.to_string_lossy().to_string())
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
    PathBuf::from(crate::system::env::expand_env(raw))
}

fn hash_file_name(id: &str) -> String {
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// 裁掉透明边，再居中放到正方形画布，避免列表里显得过小。
pub(crate) fn crop_and_fill(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut min_x = w;
    let mut max_x = 0u32;
    let mut min_y = h;
    let mut max_y = 0u32;
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y);
            if p.0[3] > 8 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        // 全透明，退化为原图缩放
        return image::imageops::resize(img, 64, 64, image::imageops::FilterType::Triangle);
    }

    let cw = max_x - min_x + 1;
    let ch = max_y - min_y + 1;
    let crop = image::imageops::crop_imm(img, min_x, min_y, cw, ch).to_image();

    let side = 64u32;
    let inner = ((side as f32) * 0.9).round() as u32;
    let scale = (inner as f32 / cw.max(ch) as f32).min(1.0);
    let nw = ((cw as f32) * scale).round().max(1.0) as u32;
    let nh = ((ch as f32) * scale).round().max(1.0) as u32;
    let resized = image::imageops::resize(&crop, nw, nh, image::imageops::FilterType::Triangle);

    let mut canvas = RgbaImage::from_pixel(side, side, image::Rgba([0, 0, 0, 0]));
    let ox = (side.saturating_sub(nw)) / 2;
    let oy = (side.saturating_sub(nh)) / 2;
    image::imageops::overlay(&mut canvas, &resized, ox as i64, oy as i64);
    canvas
}

pub(crate) fn encode_png(img: &RgbaImage) -> Option<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
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

    #[test]
    fn crop_keeps_content_centered() {
        // 8x8 全透明画布中心画 2x2 不透明块 → 裁边后居中放大到 64
        let mut img = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
        for y in 3..5 {
            for x in 3..5 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let out = crop_and_fill(&img);
        assert_eq!(out.dimensions(), (64, 64));
        assert!(out.get_pixel(32, 32).0[3] > 8, "中心应不透明");
        assert!(out.get_pixel(1, 1).0[3] <= 8, "角落应透明");
    }

    #[test]
    fn crop_all_transparent_falls_back() {
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
        let out = crop_and_fill(&img);
        assert_eq!(out.dimensions(), (64, 64));
    }
}
