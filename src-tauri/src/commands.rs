//! React 与 Rust 的 IPC 面。业务逻辑放在同级模块，本文件只做转发。

use tauri::{AppHandle, State};

use crate::model::SearchResult;
use crate::state::{rebuild_index, AppState};
use crate::{app, search, system};

#[tauri::command]
pub fn search_apps(query: String, state: State<'_, AppState>) -> Result<Vec<SearchResult>, String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    Ok(search::search(&index.apps, &query))
}

#[tauri::command]
pub fn launch_app(id: String, app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    let item = index
        .apps
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "app not found".to_string())?;
    app::launch(item)?;
    system::window::hide(&app);
    Ok(())
}

#[tauri::command]
pub fn rescan_apps(app: AppHandle) -> Result<usize, String> {
    rebuild_index(&app)
}

#[tauri::command]
pub fn toggle_window(app: AppHandle) {
    system::window::toggle(&app);
}
