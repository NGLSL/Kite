//! Kite 启动入口：只做模块装配，业务不写在本文件。

mod app;
mod commands;
mod history;
mod model;
mod search;
mod state;
mod storage;
mod system;

use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        system::window::toggle(app);
                    }
                })
                .build(),
        )
        .setup(setup)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(false) = event {
                if window.label() != "main" {
                    return;
                }
                let hide = window
                    .app_handle()
                    .try_state::<AppState>()
                    .map(|s| {
                        s.history
                            .lock()
                            .map(|h| h.load_settings().hide_on_blur)
                            .unwrap_or(true)
                    })
                    .unwrap_or(true);
                if hide {
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_apps,
            commands::launch_app,
            commands::rescan_apps,
            commands::toggle_window,
            commands::get_settings,
            commands::save_settings,
            commands::list_user_aliases,
            commands::set_user_alias,
            commands::remove_user_alias
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = state::history_db_path(app.handle());
    let history_db = storage::HistoryDb::open(&db_path)
        .map_err(|e| format!("open history db: {e}"))?;
    app.manage(AppState::new(history_db));

    if let Err(e) = system::tray::setup(app.handle()) {
        eprintln!("tray setup failed: {e}");
    }

    // 启动时若设置了开机启动，保持注册表同步
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(hdb) = state.history.lock() {
            let s = hdb.load_settings();
            if s.autostart {
                let _ = system::autostart::set_autostart(true);
            }
        }
    }

    // 扫描放工作线程；首屏只等快速索引，图标后台补
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        if let Err(e) = state::rebuild_index(&handle) {
            eprintln!("scan failed: {e}");
        }
    });

    let alt_space = Shortcut::new(Some(Modifiers::ALT), Code::Space);
    // 已被其它程序或旧实例占用时不要让整个应用崩掉
    match app.global_shortcut().register(alt_space) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Alt+Space 注册失败（可能被占用）: {e}；可从托盘打开 Kite");
            let _ = app.global_shortcut().unregister(alt_space);
            if app.global_shortcut().register(alt_space).is_err() {
                eprintln!("Alt+Space 仍不可用，请释放全局热键后重试");
            }
        }
    }
    Ok(())
}
