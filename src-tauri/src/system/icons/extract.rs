//! 底层 Win32 图标提取。unsafe GDI/Shell 代码集中隔离在本文件。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use image::RgbaImage;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::Shell::{ExtractIconExW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON};

/// 从 .exe / .ico 路径提取 PNG；任一步失败返回 None。
pub fn extract_icon_from_file(path: &Path) -> Option<Vec<u8>> {
    unsafe { extract_icon_from_file_win32(path) }
}

unsafe fn extract_icon_from_file_win32(path: &Path) -> Option<Vec<u8>> {
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

    let mut pixels = vec![0u8; (width * 4) as usize * height as usize];
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
    let img = crop_and_fill(&img);

    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

/// 裁掉透明边，再居中放到正方形画布，避免列表里显得过小。
fn crop_and_fill(img: &RgbaImage) -> RgbaImage {
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

    // 目标 64，图标占约 90%
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
