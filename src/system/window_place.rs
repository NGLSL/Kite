//! 窗口定位：按当前光标所在显示器的工作区放置启动面板。

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// 返回 (x, y)：光标所在 monitor 工作区水平居中、垂直约 1/3。
/// 失败时返回 None，调用方保持原位置。
pub fn position_on_cursor_monitor(window_w: f32, window_h: f32) -> Option<(f32, f32)> {
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
        let work = info.rcWork;
        let work_w = (work.right - work.left) as f32;
        let work_h = (work.bottom - work.top) as f32;
        if work_w <= 0.0 || work_h <= 0.0 {
            return None;
        }
        let x = work.left as f32 + ((work_w - window_w) / 2.0).max(0.0);
        let y = work.top as f32 + ((work_h - window_h) / 3.0).max(0.0);
        Some((x, y))
    }
}

#[cfg(test)]
mod tests {
    // 真实多显示器定位依赖桌面会话，Windows 实机验收；此处只保证 API 可链接。
    #[test]
    fn position_helper_links() {
        let _ = super::position_on_cursor_monitor(640.0, 420.0);
    }
}
