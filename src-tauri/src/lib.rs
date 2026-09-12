//! Kite 启动入口：只做模块装配，业务不写在本文件。

mod app;
mod commands;
mod history;
mod model;
mod search;
mod state;
mod storage;
mod system;

use tauri::{Emitter, Manager};
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
        .invoke_handler(tauri::generate_handler![
            commands::search_apps,
            commands::launch_app,
            commands::rescan_apps,
            commands::toggle_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = state::history_db_path(app.handle());
    let history_db = storage::HistoryDb::open(&db_path)
        .map_err(|e| format!("open history db: {e}"))?;
    app.manage(AppState::new(history_db));

    // 扫描放工作线程，避免拖慢窗口显示
    let handle = app.handle().clone();
    std::thread::spawn(move || match state::rebuild_index(&handle) {
        Ok(n) => {
            let _ = handle.emit("kite://index-ready", n);
        }
        Err(e) => eprintln!("scan failed: {e}"),
    });

    let alt_space = Shortcut::new(Some(Modifiers::ALT), Code::Space);
    app.global_shortcut().register(alt_space)?;
    Ok(())
}
