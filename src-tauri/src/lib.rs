use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

mod icons;
mod launcher;
mod model;
mod scanner;
mod search;
mod window;

use model::SearchResult;
use scanner::AppIndex;

pub struct AppState {
    index: Mutex<AppIndex>,
}

fn icon_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_cache_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("icons")
}

fn rebuild_index(app: &AppHandle) -> Result<usize, String> {
    let dir = icon_dir(app);
    let index = scanner::scan_apps(&dir);
    let count = index.apps.len();
    if let Some(state) = app.try_state::<AppState>() {
        *state.index.lock().map_err(|e| e.to_string())? = index;
    }
    Ok(count)
}

#[tauri::command]
fn search_apps(query: String, state: State<'_, AppState>) -> Result<Vec<SearchResult>, String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    Ok(search::search(&index.apps, &query))
}

#[tauri::command]
fn launch_app(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    let item = index
        .apps
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "app not found".to_string())?;
    launcher::launch(item)?;
    window::hide(&app);
    Ok(())
}

#[tauri::command]
fn rescan_apps(app: AppHandle) -> Result<usize, String> {
    rebuild_index(&app)
}

#[tauri::command]
fn toggle_window(app: AppHandle) {
    window::toggle(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        window::toggle(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(AppState {
                index: Mutex::new(AppIndex::empty()),
            });

            // Initial scan off the main thread so startup stays snappy
            let scan_handle = handle.clone();
            std::thread::spawn(move || match rebuild_index(&scan_handle) {
                Ok(n) => {
                    let _ = scan_handle.emit("kite://index-ready", n);
                }
                Err(e) => {
                    eprintln!("scan failed: {e}");
                }
            });

            let alt_space = Shortcut::new(Some(Modifiers::ALT), Code::Space);
            app.global_shortcut().register(alt_space)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_apps,
            launch_app,
            rescan_apps,
            toggle_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
