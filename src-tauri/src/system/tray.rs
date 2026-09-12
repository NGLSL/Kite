//! 系统托盘：打开 / 重扫 / 退出。设置入口由前端窗口负责。

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter};

use crate::state;
use crate::system::window;

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开 Kite", true, None::<&str>)?;
    let rescan = MenuItem::with_id(app, "rescan", "重新扫描应用", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &rescan, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("app icon missing from bundle");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Kite")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => window::show_and_focus(app),
            "rescan" => {
                let handle = app.clone();
                std::thread::spawn(move || {
                    let _ = state::rebuild_index(&handle);
                    let _ = handle.emit_index_ready();
                });
            }
            "settings" => {
                window::show_and_focus(app);
                let _ = app.emit_to("main", "kite://open-settings", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                window::show_and_focus(app);
            }
        })
        .build(app)?;
    Ok(())
}

trait EmitReady {
    fn emit_index_ready(&self) -> tauri::Result<()>;
}

impl EmitReady for AppHandle {
    fn emit_index_ready(&self) -> tauri::Result<()> {
        use tauri::Emitter;
        self.emit("kite://index-ready", 0)
    }
}
