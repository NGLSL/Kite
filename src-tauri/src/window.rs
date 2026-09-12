use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

pub fn show_and_focus(app: &AppHandle) {
    if let Some(win) = main_window(app) {
        let _ = win.show();
        // Center on current cursor monitor when possible
        if let Ok(monitor) = win.current_monitor() {
            if let Some(m) = monitor {
                let screen = m.size();
                let scale = m.scale_factor();
                let width = (640.0 * scale) as u32;
                let height = (420.0 * scale) as u32;
                let x = (screen.width.saturating_sub(width)) / 2;
                // Slightly above vertical center — launcher feel
                let y = (screen.height.saturating_sub(height)) / 3;
                let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
            }
        }
        let _ = win.set_focus();
        let _ = win.emit("kite://focus-search", ());
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = main_window(app) {
        let _ = win.hide();
        let _ = win.emit("kite://cleared", ());
    }
}

pub fn toggle(app: &AppHandle) {
    if let Some(win) = main_window(app) {
        let visible = win.is_visible().unwrap_or(false);
        if visible {
            hide(app);
        } else {
            show_and_focus(app);
        }
    }
}
