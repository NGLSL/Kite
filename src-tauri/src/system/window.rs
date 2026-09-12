//! 主启动器窗口：显示、隐藏、聚焦、切换。

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const WINDOW_LABEL: &str = "main";
const WINDOW_W: f64 = 640.0;
const WINDOW_H: f64 = 420.0;

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

pub fn show_and_focus(app: &AppHandle) {
    let Some(win) = main_window(app) else {
        return;
    };
    let _ = win.show();
    place_on_current_monitor(&win);
    let _ = win.set_focus();
    let _ = win.emit("kite://focus-search", ());
}

pub fn hide(app: &AppHandle) {
    let Some(win) = main_window(app) else {
        return;
    };
    let _ = win.hide();
    let _ = win.emit("kite://cleared", ());
}

pub fn toggle(app: &AppHandle) {
    let Some(win) = main_window(app) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide(app);
    } else {
        show_and_focus(app);
    }
}

fn place_on_current_monitor(win: &WebviewWindow) {
    let Ok(monitor) = win.current_monitor() else {
        return;
    };
    let Some(m) = monitor else {
        return;
    };
    let screen = m.size();
    let scale = m.scale_factor();
    let width = (WINDOW_W * scale) as u32;
    let height = (WINDOW_H * scale) as u32;
    let x = (screen.width.saturating_sub(width)) / 2;
    // 略高于垂直中心，更接近启动器习惯位置
    let y = (screen.height.saturating_sub(height)) / 3;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}
