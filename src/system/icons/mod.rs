//! 提取并缓存 Windows 应用图标。任何失败都不得阻塞搜索。
//! 按源类型分流（参考 ZeroLaunch-rs）：图片文件直接读内容、ico 取 256 大图、
//! 其余走 shell 提取；纯图片处理逻辑放本文件，Win32 调用集中在 extract.rs。

mod extract;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use image::RgbaImage;
use sha2::{Digest, Sha256};

static VALIDATED_ICON_CACHE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// 将图标 PNG 缓存到 `icon_dir`，返回绝对路径。
/// `icon_src` 为首选源（lnk 的 icon_location 可为 `path,index`）；
/// `target_fallback` 为启动 target，优先源失败或无效时再试。
pub fn cache_icon(
    icon_dir: &Path,
    id: &str,
    icon_src: Option<&str>,
    target_fallback: Option<&str>,
) -> Option<String> {
    let candidates = collect_candidates(icon_src, target_fallback);
    if candidates.is_empty() {
        return None;
    }
    std::fs::create_dir_all(icon_dir).ok()?;
    // Schema bump：策略修正后让旧版「内容有效但语义错误」的 PNG 自动重提。
    let out = icon_dir.join(format!("{}.png", hash_file_name(&format!("v2:{id}"))));
    if out.exists() {
        if !source_newer_than_cache(&out, &candidates) && is_trusted_cached_icon(&out) {
            return Some(out.to_string_lossy().to_string());
        }
        let _ = std::fs::remove_file(&out);
    }
    for (src_path, index) in &candidates {
        // `shell:` targets are virtual namespace paths, so Path::exists() is
        // false even though the Shell API can resolve and render them.
        if !src_path.exists() && !extract::is_virtual_shell_path(src_path) {
            continue;
        }
        if let Some(png) = extract_any(src_path, *index) {
            if std::fs::write(&out, &png).is_ok() {
                remember_cached_icon(&out);
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
fn extract_any(path: &Path, index: Option<i32>) -> Option<Vec<u8>> {
    match extract::classify(path) {
        extract::SourceKind::Image => decode_image_file(path),
        extract::SourceKind::Ico => {
            extract::extract_ico_large(path).or_else(|| extract::extract_shell_icon_indexed(path, index))
        }
        extract::SourceKind::Shell => extract::extract_shell_icon_indexed(path, index),
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
        if is_trusted_cached_icon(&out) {
            return Some(out.to_string_lossy().to_string());
        }
        let _ = std::fs::remove_file(&out);
    }
    let png = extract::extract_ext_icon(key, is_dir)?;
    std::fs::write(&out, &png).ok()?;
    remember_cached_icon(&out);
    Some(out.to_string_lossy().to_string())
}

/// A previous build may have left a truncated or all-transparent PNG behind.
/// Do not keep returning that path forever; let the caller extract it again.
fn is_usable_cached_icon(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(img) = image::load_from_memory(&bytes) else {
        return false;
    };
    img.to_rgba8().pixels().any(|p| p.0[3] > 8)
}

/// 安装包更新 exe 或 UWP logo 后，在下一次索引扫描时重提图标。
/// 虚拟 Shell 目标没有文件修改时间，继续使用有效的现有缓存。
fn source_newer_than_cache(cache: &Path, candidates: &[(PathBuf, Option<i32>)]) -> bool {
    let Ok(cached_at) = std::fs::metadata(cache).and_then(|m| m.modified()) else {
        return true;
    };
    candidates.iter().any(|(source, _)| {
        std::fs::metadata(source)
            .and_then(|m| m.modified())
            .is_ok_and(|modified| modified > cached_at)
    })
}

/// Validate an existing cache file at most once per process. Search results
/// call `cache_icon` on every keystroke, so decoding the same PNG on every hit
/// would put disk and image work back on the search path.
fn is_trusted_cached_icon(path: &Path) -> bool {
    let validated = VALIDATED_ICON_CACHE.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(g) = validated.lock() {
        if g.contains(path) {
            return true;
        }
    }
    if !is_usable_cached_icon(path) {
        return false;
    }
    remember_cached_icon(path);
    true
}

fn remember_cached_icon(path: &Path) {
    let validated = VALIDATED_ICON_CACHE.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut g) = validated.lock() {
        g.insert(path.to_path_buf());
    }
}

/// 图标源候选：首选 icon_location（`path,index`），再追加启动 target 回退。
fn collect_candidates(icon_src: Option<&str>, target_fallback: Option<&str>) -> Vec<(PathBuf, Option<i32>)> {
    let mut list = Vec::new();
    let mut push_unique = |raw: &str, list: &mut Vec<(PathBuf, Option<i32>)>| {
        let s = raw.trim().trim_matches('"');
        if s.is_empty() {
            return;
        }
        let (path, index) = split_icon_index(s);
        let expanded = expand_env_path(&path);
        if list.iter().any(|(p, _)| p == &expanded) {
            return;
        }
        list.push((expanded, index));
    };
    if let Some(s) = icon_src {
        push_unique(s, &mut list);
    }
    if let Some(t) = target_fallback {
        // 回退 target 不带资源序号
        let s = t.trim().trim_matches('"');
        if !s.is_empty() {
            let expanded = expand_env_path(s);
            if !list.iter().any(|(p, _)| p == &expanded) {
                list.push((expanded, None));
            }
        }
    }
    list
}

/// 拆出 icon_location 的资源索引后缀：`shell32.dll,-34` → ("shell32.dll", Some(-34))。
/// 负数是资源 ID（Windows lnk 惯例）；逗号后不是数字则视为路径本身含逗号。
fn split_icon_index(src: &str) -> (String, Option<i32>) {
    match src.rsplit_once(',') {
        Some((path, idx)) => match idx.trim().parse::<i32>() {
            Ok(v) => (path.to_string(), Some(v)),
            Err(_) => (src.to_string(), None),
        },
        None => (src.to_string(), None),
    }
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
    fn split_icon_index_variants() {
        assert_eq!(
            split_icon_index(r"C:\a\shell32.dll,-34"),
            (r"C:\a\shell32.dll".to_string(), Some(-34))
        );
        assert_eq!(
            split_icon_index(r"C:\a\app.exe,2"),
            (r"C:\a\app.exe".to_string(), Some(2))
        );
        assert_eq!(
            split_icon_index(r"C:\a\app.exe"),
            (r"C:\a\app.exe".to_string(), None)
        );
        // 逗号后不是数字：视为路径本身含逗号，保持原样
        assert_eq!(
            split_icon_index(r"C:\a,b\app.exe"),
            (r"C:\a,b\app.exe".to_string(), None)
        );
    }

    #[test]
    fn collect_candidates_prefer_icon_src_then_target() {
        let c = collect_candidates(
            Some(r"C:\app\ProductIcon,3"),
            Some(r"C:\app\App.exe"),
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].0.to_string_lossy(), r"C:\app\ProductIcon");
        assert_eq!(c[0].1, Some(3));
        assert_eq!(c[1].0.to_string_lossy(), r"C:\app\App.exe");
        assert_eq!(c[1].1, None);

        // 同路径不重复
        let c = collect_candidates(Some(r"C:\app\app.exe,0"), Some(r"C:\app\app.exe"));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1, Some(0));
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

    #[test]
    #[cfg(windows)]
    fn cache_shell_namespace_icon() {
        let dir = std::env::temp_dir().join(format!("kite-shell-icon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cached = cache_icon(&dir, "shell:recycle-bin", Some("shell:RecycleBinFolder"), None);
        assert!(cached.is_some(), "shell namespace icon should be extracted");
        let path = cached.unwrap();
        let bytes = std::fs::read(&path).expect("cached shell icon");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        for (id, target) in [
            ("control", "shell:ControlPanelFolder"),
            ("connections", "shell:ConnectionsFolder"),
            ("printers", "shell:PrintersFolder"),
            ("admin", "shell:Administrative Tools"),
        ] {
            let cached = cache_icon(&dir, id, Some(target), None).expect("shell target icon");
            let bytes = std::fs::read(cached).expect("cached shell target icon");
            let img = image::load_from_memory(&bytes)
                .expect("shell target PNG")
                .to_rgba8();
            assert!(
                img.pixels().any(|p| p.0[3] > 8),
                "shell target returned an all-transparent icon: {target}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(windows)]
    fn transparent_cached_icon_is_rebuilt() {
        let dir = std::env::temp_dir().join(format!("kite-bad-icon-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = "transparent-cache";
        let out = dir.join(format!("{}.png", hash_file_name(&format!("v2:{id}"))));
        let blank = RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
        std::fs::write(&out, encode_png(&blank).unwrap()).unwrap();

        let source = std::env::current_exe().unwrap();
        let cached = cache_icon(&dir, id, source.to_str(), None);
        assert!(cached.is_some(), "invalid cached icon should be extracted again");
        let bytes = std::fs::read(cached.unwrap()).unwrap();
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert!(img.pixels().any(|p| p.0[3] > 8));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_icon_source_replaces_cached_png() {
        let dir = std::env::temp_dir().join(format!("kite-icon-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        let red = RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]));
        std::fs::write(&source, encode_png(&red).unwrap()).unwrap();
        let cached = cache_icon(&dir, "changing-source", source.to_str(), None).unwrap();

        let blue = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 255, 255]));
        std::fs::write(&source, encode_png(&blue).unwrap()).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&cached)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();

        let refreshed = cache_icon(&dir, "changing-source", source.to_str(), None).unwrap();
        let image = image::open(refreshed).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(32, 32).0, [0, 0, 255, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
