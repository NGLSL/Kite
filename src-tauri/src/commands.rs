//! React 与 Rust 的 IPC 面。业务逻辑放在同级模块，本文件只做转发。

use tauri::{AppHandle, State};

use crate::model::SearchResult;
use crate::state::{rebuild_index, AppState};
use crate::{app, history, search, storage, system};

#[tauri::command]
pub fn search_apps(query: String, state: State<'_, AppState>) -> Result<Vec<SearchResult>, String> {
    let q_norm = search::normalize_for_index(&query);
    let index = state.index.lock().map_err(|e| e.to_string())?;
    let mut hits = search::search(&index.apps, &query);

    // 历史加权：仅给已召回的条目加分，Match 仍是主信号
    if !q_norm.is_empty() {
        if let Ok(hdb) = state.history.lock() {
            let now = storage::now_ts();
            for hit in &mut hits {
                let usage = hdb.usage(&hit.item.id).unwrap_or_default();
                let pair = hdb.query_pair(&q_norm, &hit.item.id).unwrap_or_default();
                let boost = history::history_boost(&q_norm, &usage, &pair, now);
                hit.score += boost;
                if boost > 0 {
                    hit.matched_by = format!("{}+history", hit.matched_by);
                }
            }
            hits = search::rerank(hits, search::TOP_N);
        }
    }

    Ok(hits)
}

#[tauri::command]
pub fn launch_app(
    id: String,
    query: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    let item = index
        .apps
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "app not found".to_string())?;
    let id = item.id.clone();
    app::launch(item)?;

    let q_norm = search::normalize_for_index(&query);
    if let Ok(mut hdb) = state.history.lock() {
        let now = storage::now_ts();
        if let Err(e) = hdb.record_launch(&id, &q_norm, now) {
            eprintln!("history write failed: {e}");
        }
    }

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
