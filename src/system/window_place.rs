//! 窗口定位：按当前光标所在显示器的工作区放置启动面板。
//!
//! Win32 的 `GetCursorPos` / `MONITORINFO.rcWork` 是**物理像素**；
//! iced `window::move_to` 收的是**逻辑坐标**（winit 再乘上窗口当前 scale）。
//! 混用两者在 100% 缩放下碰巧正确，125%/150%（常见 2K）会整体偏移。

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// 光标所在 monitor 上，逻辑窗口 `window_w × window_h` 居中后的**物理**左上角。
/// 水平、垂直均按工作区居中。失败返回 None，调用方保持原位置。
pub fn physical_position_on_cursor_monitor(
    window_w_logical: f32,
    window_h_logical: f32,
) -> Option<(f32, f32)> {
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = if dpi_x == 0 {
            1.0
        } else {
            dpi_x as f32 / 96.0
        };
        let work = info.rcWork;
        Some(center_physical(
            work.left as f32,
            work.top as f32,
            work.right as f32,
            work.bottom as f32,
            window_w_logical,
            window_h_logical,
            scale,
        ))
    }
}

/// 工作区物理矩形内居中逻辑尺寸窗口，返回物理左上角。
fn center_physical(
    work_left: f32,
    work_top: f32,
    work_right: f32,
    work_bottom: f32,
    window_w_logical: f32,
    window_h_logical: f32,
    scale: f32,
) -> (f32, f32) {
    let work_w = work_right - work_left;
    let work_h = work_bottom - work_top;
    let window_w = window_w_logical * scale;
    let window_h = window_h_logical * scale;
    let x = work_left + ((work_w - window_w) / 2.0).max(0.0);
    let y = work_top + ((work_h - window_h) / 2.0).max(0.0);
    (x, y)
}

/// 物理坐标 → iced `move_to` 所需的逻辑坐标。
/// 用的是**窗口当前** scale（winit `set_outer_position(Logical)` 会再乘回去），
/// 这样跨不同 DPI 显示器移动时落点仍是算出的物理位置。
pub fn physical_to_logical_for_window(physical: (f32, f32), window_scale: f32) -> (f32, f32) {
    let s = if window_scale <= 0.0 { 1.0 } else { window_scale };
    (physical.0 / s, physical.1 / s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_on_100_percent_monitor() {
        // 1920×1080 工作区，640×420 逻辑窗，scale 1.0 → 水平垂直居中
        let (x, y) = center_physical(0.0, 0.0, 1920.0, 1080.0, 640.0, 420.0, 1.0);
        assert_eq!(x, 640.0);
        assert_eq!(y, 330.0);
    }

    #[test]
    fn centers_on_150_percent_monitor_using_physical_work_area() {
        // 2K 2560×1440 @150%：逻辑窗 640×420 → 物理 960×630，垂直居中
        let (x, y) = center_physical(0.0, 0.0, 2560.0, 1440.0, 640.0, 420.0, 1.5);
        assert_eq!(x, 800.0);
        assert_eq!(y, 405.0);
    }

    #[test]
    fn secondary_monitor_keeps_physical_origin() {
        // 副屏物理原点在 x=1920，不得再被 scale 除到主屏
        let (x, y) = center_physical(1920.0, 0.0, 4480.0, 1440.0, 640.0, 420.0, 1.5);
        assert_eq!(x, 1920.0 + 800.0);
        assert_eq!(y, 405.0);
    }

    #[test]
    fn logical_conversion_uses_window_scale_not_target_scale() {
        // 窗口仍在主屏 scale=1.0，目标是 2K 副屏物理 (2720, 270)
        let (lx, ly) = physical_to_logical_for_window((2720.0, 270.0), 1.0);
        assert_eq!((lx, ly), (2720.0, 270.0));

        // 窗口已在 2K（scale=1.5）时，同一物理点对应的逻辑值更小
        let (lx, ly) = physical_to_logical_for_window((2720.0, 270.0), 1.5);
        assert!((lx - 2720.0 / 1.5).abs() < 0.01);
        assert!((ly - 270.0 / 1.5).abs() < 0.01);
    }

    #[test]
    fn position_helper_links() {
        let _ = super::physical_position_on_cursor_monitor(640.0, 420.0);
    }
}
