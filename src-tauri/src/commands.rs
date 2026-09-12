//! React 与 Rust 的 IPC 面。业务逻辑放在同级模块，本文件只做转发。

use tauri::{AppHandle, Emitter, State};

use crate::model::{AppItem, SearchResult};
use crate::state::{rebuild_index, AppState};
use crate::storage::settings::{Settings, UserAlias};
use crate::{app, history, search, storage, system};

#[tauri::command]
pub fn search_apps(query: String, state: State<'_, AppState>) -> Result<Vec<SearchResult>, String> {
    let q_norm = search::normalize_for_index(&query);
    let index = state.index.lock().map_err(|e| e.to_string())?;

    let user_targets: Vec<String> = state
        .history
        .lock()
        .map(|h| {
            h.alias_targets(&q_norm)
                .into_iter()
                .map(|t| t.to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    let mut hits = search::search(&index.apps, &query, &user_targets);

    // Everything 文件搜索：应用结果排前，文件补尾
    if !q_norm.is_empty() {
        let max_files = 5usize;
        let file_hits = system::everything::search_files(&query, max_files);
        for fh in file_hits {
            let name = fh.name.clone();
            let id = format!("file:{}", fh.path.to_lowercase());
            let mut item = AppItem::scanned(id, name, fh.path, None, None, "everything");
            item.attach_search_fields();
            hits.push(SearchResult {
                item,
                score: 400,
                matched_by: "file".into(),
            });
        }
    }

    // 历史加权（Match 仍是主信号）
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
    } else {
        hits = search::rerank(hits, search::TOP_N);
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
    // Everything 文件结果：直接按路径打开
    if let Some(path) = id.strip_prefix("file:") {
        let path = path.replace('/', "\\");
        app::launch(&AppItem::scanned(
            id.clone(),
            path.clone(),
            path,
            None,
            None,
            "everything",
        ))?;
        let q_norm = search::normalize_for_index(&query);
        if let Ok(mut hdb) = state.history.lock() {
            let now = storage::now_ts();
            let _ = hdb.record_launch(&id, &q_norm, now);
        }
        system::window::hide(&app);
        return Ok(());
    }

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

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let hdb = state.history.lock().map_err(|e| e.to_string())?;
    Ok(hdb.load_settings())
}

#[tauri::command]
pub fn save_settings(
    hide_on_blur: Option<bool>,
    autostart: Option<bool>,
    max_results: Option<i64>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Settings, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    let mut s = hdb.load_settings();
    if let Some(v) = hide_on_blur {
        s.hide_on_blur = v;
        let _ = hdb.save_setting("hide_on_blur", if v { "1" } else { "0" });
        if let Some(win) = system::window::main_window(&app) {
            win.set_always_on_top(true).ok();
        }
    }
    if let Some(v) = autostart {
        s.autostart = v;
        let _ = hdb.save_setting("autostart", if v { "1" } else { "0" });
        system::autostart::set_autostart(v)?;
    }
    if let Some(v) = max_results {
        let v = v.clamp(5, 30);
        s.max_results = v;
        let _ = hdb.save_setting("max_results", &v.to_string());
    }
    let _ = app.emit("kite://settings-changed", ());
    Ok(s)
}

#[tauri::command]
pub fn list_user_aliases(state: State<'_, AppState>) -> Result<Vec<UserAlias>, String> {
    state
        .history
        .lock()
        .map_err(|e| e.to_string())?
        .list_aliases()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_user_alias(
    alias: String,
    target_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<UserAlias>, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.set_alias(&alias, &target_name).map_err(|e| e.to_string())?;
    hdb.list_aliases().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_user_alias(
    alias: String,
    state: State<'_, AppState>,
) -> Result<Vec<UserAlias>, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.remove_alias(&alias).map_err(|e| e.to_string())?;
    hdb.list_aliases().map_err(|e| e.to_string())
}
