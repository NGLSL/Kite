use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use sha2::{Digest, Sha256};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::Shell::{ExtractIconExW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON};

fn hash_file_name(id: &str) -> String {
    let mut h = Sha256::new();
    h.update(id.as_bytes());
    h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Convert an HICON to PNG bytes.
unsafe fn hicon_to_png(hicon: HICON) -> Option<Vec<u8>> {
    let mut info = std::mem::zeroed();
    if GetIconInfo(hicon, &mut info).is_err() {
        return None;
    }
    let color_bmp = info.hbmColor;
    let mask_bmp = info.hbmMask;

    let mut bmp = std::mem::zeroed::<windows::Win32::Graphics::Gdi::BITMAP>();
    let ok = windows::Win32::Graphics::Gdi::GetObjectW(
        HGDIOBJ(color_bmp.0),
        std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAP>() as i32,
        Some(&mut bmp as *mut _ as *mut _),
    );
    if ok == 0 {
        let _ = DeleteObject(HGDIOBJ(color_bmp.0));
        let _ = DeleteObject(HGDIOBJ(mask_bmp.0));
        return None;
    }

    let width = bmp.bmWidth;
    let height = bmp.bmHeight;
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

    let stride = (width * 4) as usize;
    let mut pixels = vec![0u8; stride * height as usize];
    let selected = SelectObject(mem_dc, HGDIOBJ(color_bmp.0));
    let got = GetDIBits(
        mem_dc,
        color_bmp,
        0,
        height as u32,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    let _ = SelectObject(mem_dc, selected);
    let _ = DeleteDC(mem_dc);
    let _ = ReleaseDC(Some(HWND::default()), hdc_screen);
    let _ = DeleteObject(HGDIOBJ(color_bmp.0));
    let _ = DeleteObject(HGDIOBJ(mask_bmp.0));

    if got == 0 {
        return None;
    }

    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }

    let img = RgbaImage::from_raw(width as u32, height as u32, pixels)?;
    let img = if img.width() > 128 {
        image::imageops::resize(&img, 64, 64, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

unsafe fn extract_icon_from_file(path: &Path) -> Option<Vec<u8>> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut shfi = std::mem::zeroed::<SHFILEINFOW>();
    let ok = SHGetFileInfoW(
        windows::core::PCWSTR(wide.as_ptr()),
        Default::default(),
        Some(&mut shfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON,
    );
    if ok != 0 && !shfi.hIcon.is_invalid() {
        let png = hicon_to_png(shfi.hIcon);
        let _ = DestroyIcon(shfi.hIcon);
        if png.is_some() {
            return png;
        }
    }

    let mut large = [HICON::default(); 1];
    let n = ExtractIconExW(
        windows::core::PCWSTR(wide.as_ptr()),
        0,
        Some(large.as_mut_ptr()),
        None,
        1,
    );
    if n > 0 && !large[0].is_invalid() {
        let png = hicon_to_png(large[0]);
        let _ = DestroyIcon(large[0]);
        return png;
    }
    None
}

/// Cache an icon PNG under `icon_dir` and return its absolute path.
/// Failures return None so search is never blocked by icon issues.
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
    let png = unsafe { extract_icon_from_file(&src_path) }?;
    std::fs::write(&out, &png).ok()?;
    Some(out.to_string_lossy().to_string())
}
