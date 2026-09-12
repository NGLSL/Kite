//! 主启动器窗口：显示、隐藏、聚焦、切换。

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const WINDOW_LABEL: &str = "main";
const WINDOW_W: f64 = 640.0;
const WINDOW_H: f64 = 420.0;
const SETTINGS_W: f64 = 720.0;
const SETTINGS_H: f64 = 520.0;

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

/// 切换搜索/设置尺寸（逻辑像素，避免 DPI 换算问题）。
pub fn set_settings_mode(app: &AppHandle, settings: bool) {
    let Some(win) = main_window(app) else {
        return;
    };
    let (w, h) = if settings {
        (SETTINGS_W, SETTINGS_H)
    } else {
        (WINDOW_W, WINDOW_H)
    };
    match win.set_size(tauri::LogicalSize::new(w, h)) {
        Ok(()) => crate::log::info(&format!("window size -> {w}x{h} settings={settings}")),
        Err(e) => crate::log::info(&format!("set_size failed: {e}")),
    }
    place_on_current_monitor(&win);
}

pub fn show_and_focus(app: &AppHandle) {
    // 唤起即响（快捷键/托盘统一走这里；音效异步不阻塞显示）
    crate::system::sound::play_open();
    let Some(win) = main_window(app) else {
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    // 设置页保持大窗；仅在纯搜索模式时保证搜索尺寸
    let settings_open = app
        .try_state::<crate::state::AppState>()
        .map(|s| s.is_settings_open())
        .unwrap_or(false);
    if !settings_open {
        set_settings_mode(app, false);
    }
    let _ = win.set_focus();
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(16));
        if let Some(w) = main_window(&handle) {
            let _ = w.set_focus();
            if !settings_open {
                let _ = w.emit("kite://focus-search", ());
            }
        }
    });
}

pub fn hide(app: &AppHandle) {
    let Some(win) = main_window(app) else {
        return;
    };
    // 设置页打开时禁止隐藏（失焦点开关等不应收起）
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        if state.is_settings_open() {
            return;
        }
        state.set_settings_open(false);
    }
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
    let size = win.outer_size().unwrap_or(tauri::PhysicalSize::new(
        (WINDOW_W * scale) as u32,
        (WINDOW_H * scale) as u32,
    ));
    let x = (screen.width.saturating_sub(size.width)) / 2;
    let y = (screen.height.saturating_sub(size.height)) / 3;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}
