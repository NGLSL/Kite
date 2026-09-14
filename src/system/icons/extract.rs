//! 底层 Win32 图标提取。unsafe GDI/Shell 代码集中隔离在本文件。
//! 兼容多种情况：SHGetFileInfo 大图标 → ExtractIconEx → 蒙板/全透明兜底。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    ExtractIconExW, SHGetFileInfoW, SHGetStockIconInfo, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
    SHGFI_USEFILEATTRIBUTES, SHGSI_ICON,
    SHGSI_LARGEICON, SHSTOCKICONID, SHSTOCKICONINFO, SIID_MYNETWORK, SIID_NETWORKCONNECT,
    SIID_PRINTER, SIID_RECYCLER, SIID_SOFTWARE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GetIconInfo, LoadImageW, HICON, IMAGE_ICON, LR_LOADFROMFILE,
};

/// 图标源类型；决定提取策略（mod.rs 分流）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// PNG/JPG 等图片文件：内容即图标（UWP logo）。
    Image,
    /// .ico：可请求 256×256 大图。
    Ico,
    /// exe/dll/lnk 等：走 shell 提取。
    Shell,
}

pub fn classify(path: &Path) -> SourceKind {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") => SourceKind::Image,
        Some("ico") => SourceKind::Ico,
        _ => {
            // MSI 安装的快捷方式图标源常是无扩展名但文件头为 ICO（00 00 01 00）。
            if looks_like_ico_file(path) {
                return SourceKind::Ico;
            }
            SourceKind::Shell
        }
    }
}

/// 按文件头识别无扩展名 ICO（ICONDIR reserved=0, type=1）。
pub fn looks_like_ico_file(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    magic == [0x00, 0x00, 0x01, 0x00]
}

/// Shell namespace targets such as `shell:RecycleBinFolder` do not exist as
/// filesystem paths but can still be resolved by SHGetFileInfoW.
pub fn is_virtual_shell_path(path: &Path) -> bool {
    path.to_string_lossy()
        .trim()
        .to_ascii_lowercase()
        .starts_with("shell:")
}

/// shell 提取链：SHGetFileInfo 大图标 → ExtractIconEx 主图标 → 小图标。
/// 后台扫描线程可能未初始化 COM，Shell API 会失败（控制面板等虚拟命名空间尤甚）。
pub fn extract_shell_icon(path: &Path) -> Option<Vec<u8>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if is_virtual_shell_path(path) {
            // Prefer resolving the live Shell namespace object so Control Panel
            // and friends show the real system icon, not a generic stock glyph.
            if let Some(png) = extract_shell_icon_win32(path) {
                return Some(png);
            }
            if let Some(png) = stock_icon_id(path).and_then(|id| extract_stock_icon(id)) {
                return Some(png);
            }
            return extract_shell_icon_win32(path);
        }
        extract_shell_icon_win32(path)
    }
}

/// Return a stable system icon for built-in Shell namespace targets that do
/// not have a filesystem path. The stock icon API avoids showing Kite's
/// first-character placeholder when SHGetFileInfo cannot resolve a virtual
/// namespace path directly.
fn stock_icon_id(path: &Path) -> Option<SHSTOCKICONID> {
    let value = path.to_string_lossy().trim().to_ascii_lowercase();
    match value.as_str() {
        "shell:recyclebinfolder" => Some(SIID_RECYCLER),
        "shell:controlpanelfolder" => Some(SIID_SOFTWARE),
        "shell:connectionsfolder" => Some(SIID_NETWORKCONNECT),
        "shell:printersfolder" => Some(SIID_PRINTER),
        "shell:administrative tools" => Some(SIID_SOFTWARE),
        "shell:mynetworkplaces" => Some(SIID_MYNETWORK),
        _ => None,
    }
}

unsafe fn extract_stock_icon(id: SHSTOCKICONID) -> Option<Vec<u8>> {
    let mut info = SHSTOCKICONINFO {
        cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    SHGetStockIconInfo(id, SHGSI_ICON | SHGSI_LARGEICON, &mut info).ok()?;
    if info.hIcon.is_invalid() {
        return None;
    }
    let png = hicon_to_png(info.hIcon);
    let _ = DestroyIcon(info.hIcon);
    png
}

/// 带资源索引/ID 提取（lnk icon_location 的 `,N` / `,-ID` 后缀；负数是资源 ID）。
/// 先按索引提取，失败退回常规提取链兜底。
pub fn extract_shell_icon_indexed(path: &Path, index: Option<i32>) -> Option<Vec<u8>> {
    let Some(index) = index else {
        return extract_shell_icon(path);
    };
    unsafe {
        let wide = wide_path(path);
        let pcw = windows::core::PCWSTR(wide.as_ptr());
        let mut large = [HICON::default(); 1];
        let n = ExtractIconExW(pcw, index, Some(large.as_mut_ptr()), None, 1);
        if n > 0 && !large[0].is_invalid() {
            let png = hicon_to_png(large[0]);
            let _ = DestroyIcon(large[0]);
            if png.is_some() {
                return png;
            }
        }
        extract_shell_icon_win32(path)
    }
}

/// .ico 专用：按 256×256 请求（多数 ico 内含大尺寸资源）。
pub fn extract_ico_large(path: &Path) -> Option<Vec<u8>> {
    unsafe {
        let wide = wide_path(path);
        let h = LoadImageW(
            None,
            windows::core::PCWSTR(wide.as_ptr()),
            IMAGE_ICON,
            256,
            256,
            LR_LOADFROMFILE,
        )
        .ok()?;
        let hicon = HICON(h.0);
        let png = hicon_to_png(hicon);
        let _ = DestroyIcon(hicon);
        png
    }
}

/// 扩展名 → 系统关联图标（不触碰真实文件）；is_dir 时给文件夹图标。
pub fn extract_ext_icon(ext: &str, is_dir: bool) -> Option<Vec<u8>> {
    unsafe {
        let name = if is_dir {
            "folder".to_string()
        } else {
            format!("x.{ext}")
        };
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let attrs: FILE_FLAGS_AND_ATTRIBUTES = if is_dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        let mut shfi = std::mem::zeroed::<SHFILEINFOW>();
        let ok = SHGetFileInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            attrs,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
        );
        if ok == 0 || shfi.hIcon.is_invalid() {
            return None;
        }
        let png = hicon_to_png(shfi.hIcon);
        let _ = DestroyIcon(shfi.hIcon);
        png
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

unsafe fn extract_shell_icon_win32(path: &Path) -> Option<Vec<u8>> {
    let wide = wide_path(path);
    let pcw = windows::core::PCWSTR(wide.as_ptr());

    // 1) Shell 大图标（多数 exe/ico 可用）
    let mut shfi = std::mem::zeroed::<SHFILEINFOW>();
    let ok = SHGetFileInfoW(
        pcw,
        Default::default(),
        Some(&mut shfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_LARGEICON,
    );
    if ok != 0 && !shfi.hIcon.is_invalid() {
        let png = hicon_to_png(shfi.hIcon);
        let _ = DestroyIcon(shfi.hIcon);
        if png.is_some() {
            return png;
        }
    }

    // 2) ExtractIconEx 主图标（部分壳资源/旧 exe）
    let mut large = [HICON::default(); 1];
    let n = ExtractIconExW(pcw, 0, Some(large.as_mut_ptr()), None, 1);
    if n > 0 && !large[0].is_invalid() {
        let png = hicon_to_png(large[0]);
        let _ = DestroyIcon(large[0]);
        if png.is_some() {
            return png;
        }
    }

    // 3) 再试小图标（极少数只有 16x16 资源）
    let mut small = [HICON::default(); 1];
    let n = ExtractIconExW(pcw, 0, None, Some(small.as_mut_ptr()), 1);
    if n > 0 && !small[0].is_invalid() {
        let png = hicon_to_png(small[0]);
        let _ = DestroyIcon(small[0]);
        return png;
    }
    None
}

pub(crate) unsafe fn hicon_to_png(hicon: HICON) -> Option<Vec<u8>> {
    let mut info = std::mem::zeroed();
    if GetIconInfo(hicon, &mut info).is_err() {
        return None;
    }
    let color_bmp = info.hbmColor;
    let mask_bmp = info.hbmMask;

    // 颜色位图缺失时用蒙板（单色图标）
    let use_mask = color_bmp.is_invalid();
    let src_bmp = if use_mask { mask_bmp } else { color_bmp };
    if src_bmp.is_invalid() {
        if !color_bmp.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        }
        if !mask_bmp.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        }
        return None;
    }

    let mut bmp = std::mem::zeroed::<windows::Win32::Graphics::Gdi::BITMAP>();
    let ok = windows::Win32::Graphics::Gdi::GetObjectW(
        HGDIOBJ(src_bmp.0),
        std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAP>() as i32,
        Some(&mut bmp as *mut _ as *mut _),
    );
    if ok == 0 {
        let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        return None;
    }

    let width = bmp.bmWidth;
    let mut height = bmp.bmHeight;
    // 单色图标：高度是 内容+蒙板，取一半
    if use_mask && height > 0 {
        height /= 2;
    }
    if width <= 0 || height <= 0 || width > 512 || height > 512 {
        let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        return None;
    }

    let hdc_screen = GetDC(Some(HWND::default()));
    let mem_dc = CreateCompatibleDC(Some(hdc_screen));
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0 as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut pixels = vec![0u8; (width * 4) as usize * height as usize];
    let got = GetDIBits(
        mem_dc,
        src_bmp,
        0,
        height as u32,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );

    // Some legacy/color HICONs expose a color bitmap whose alpha channel is
    // entirely zero. Treating that as a valid image writes a transparent PNG
    // (the visible icon then looks like a failed load). Rebuild the alpha
    // channel from the AND mask before releasing the GDI bitmaps.
    if got != 0
        && !use_mask
        && pixels.chunks_exact(4).all(|px| px[3] <= 8)
        && !mask_bmp.is_invalid()
    {
        let mut mask_bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut mask_pixels = vec![0u8; (width * 4) as usize * height as usize];
        let got_mask = GetDIBits(
            mem_dc,
            mask_bmp,
            0,
            height as u32,
            Some(mask_pixels.as_mut_ptr() as *mut _),
            &mut mask_bmi,
            DIB_RGB_COLORS,
        );
        if got_mask != 0 {
            for (color, mask) in pixels.chunks_exact_mut(4).zip(mask_pixels.chunks_exact(4)) {
                let mask_value = mask[0].max(mask[1]).max(mask[2]);
                // AND-mask 0 means the color pixel is visible; 1 is transparent.
                color[3] = if mask_value < 128 { 255 } else { 0 };
            }
        }
    }

    let _ = DeleteDC(mem_dc);
    let _ = ReleaseDC(Some(HWND::default()), hdc_screen);
    let _ = DeleteObject(HGDIOBJ(color_bmp.0));
    let _ = DeleteObject(HGDIOBJ(mask_bmp.0));

    if got == 0 {
        return None;
    }

    // BGRA → RGBA
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    // 单色位图读出的是黑白，alpha 全 0；置为不透明并反相（黑底白图）
    if use_mask {
        for px in pixels.chunks_exact_mut(4) {
            let v = px[0];
            px[0] = 255 - v;
            px[1] = 255 - v;
            px[2] = 255 - v;
            px[3] = 255;
        }
    }

    // A malformed/legacy icon may have no usable alpha or mask. Keep useful
    // RGB visible rather than caching an all-transparent PNG; all-zero data
    // is just another extraction failure and should try the next source.
    if pixels.chunks_exact(4).all(|px| px[3] <= 8) {
        if pixels
            .chunks_exact(4)
            .all(|px| px[0] <= 8 && px[1] <= 8 && px[2] <= 8)
        {
            return None;
        }
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }

    let img = image::RgbaImage::from_raw(width as u32, height as u32, pixels)?;
    let img = super::crop_and_fill(&img);
    super::encode_png(&img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_by_extension() {
        assert_eq!(classify(Path::new("C:\\a\\Logo.scale-200.PNG")), SourceKind::Image);
        assert_eq!(classify(Path::new("C:\\a\\x.jpg")), SourceKind::Image);
        assert_eq!(classify(Path::new("C:\\a\\x.webp")), SourceKind::Image);
        assert_eq!(classify(Path::new("C:\\a\\app.ico")), SourceKind::Ico);
        assert_eq!(classify(Path::new("C:\\a\\app.EXE")), SourceKind::Shell);
        assert_eq!(classify(Path::new("C:\\a\\shell32.dll")), SourceKind::Shell);
        assert_eq!(classify(Path::new("no-ext")), SourceKind::Shell);
    }

    #[test]
    fn extensionless_ico_magic_is_classified_as_ico() {
        let dir = std::env::temp_dir().join(format!("kite-ico-magic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ProductIcon");
        // Minimal ICONDIR header: reserved=0, type=1, count=1
        std::fs::write(&path, [0x00u8, 0x00, 0x01, 0x00, 0x01, 0x00]).unwrap();
        assert!(looks_like_ico_file(&path));
        assert_eq!(classify(&path), SourceKind::Ico);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extracts_existing_executable_icon() {
        let path = std::env::current_exe().expect("test executable path");
        let png = extract_shell_icon(&path).expect("existing executable should have a shell icon");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
