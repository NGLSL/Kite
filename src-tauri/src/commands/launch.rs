//! 启动 / 动作类 IPC：启动应用、结果上下文动作、窗口与重扫。

use tauri::{AppHandle, Emitter, State};

use crate::model::AppItem;
use crate::state::{rebuild_index, AppState};
use crate::{app, search, storage, system};

#[tauri::command]
pub fn launch_app(
    id: String,
    query: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 内置动作：打开 Kite 设置（不关窗）
    if id == "kite:settings" {
        state.set_settings_open(true);
        let _ = app.emit_to("main", "kite://open-settings", ());
        system::window::set_settings_mode(&app, true);
        return Ok(());
    }

    // 启动器常驻后台，先刷新进程环境再拉起子进程（详见 system::env）
    system::env::refresh_process_env();
    // 历史已记录：清搜索缓存，同 Query 下次搜索会带上新的历史加权
    state.cache_clear();

    // 浏览器打开网址 / 网页搜索
    if let Some((kind, browser_id, payload)) = app::web::parse_id(&id) {
        let cached = state
            .history
            .lock()
            .ok()
            .and_then(|h| h.search_url_template());
        let preferred = if kind == "websearch" {
            app::web::launch_websearch(&browser_id, &payload, cached.as_deref())?
        } else {
            app::web::launch_url(&browser_id, &payload)?
        };
        let q_norm = search::normalize_for_index(&query);
        if let Ok(mut hdb) = state.history.lock() {
            let now = storage::now_ts();
            let _ = hdb.record_launch(&id, &q_norm, now);
            if let Some(pid) = preferred {
                let _ = hdb.set_preferred_browser(&pid);
            }
        }
        system::window::hide(&app);
        return Ok(());
    }

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

/// 结果上下文动作：open_folder / pin / unpin。
/// 复制路径与复制名称是纯剪贴板写，在前端完成。
#[tauri::command]
pub fn result_action(
    id: String,
    action: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    match action.as_str() {
        "open_folder" => {
            let target = resolve_target(&state, &id)?;
            app::actions::open_containing_folder(&target)
        }
        "pin" => {
            let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
            hdb.pin_item(&id, storage::now_ts())
                .map_err(|e| e.to_string())?;
            drop(hdb);
            // 空 Query 的固定优先排序依赖缓存，固定后立即失效
            state.cache_clear();
            Ok(())
        }
        "unpin" => {
            let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
            hdb.unpin_item(&id).map_err(|e| e.to_string())?;
            drop(hdb);
            state.cache_clear();
            Ok(())
        }
        other => Err(format!("未知动作: {other}")),
    }
}

/// 把结果 id 解析成真实目标：索引项按 id 查，Everything 结果 id 即路径。
fn resolve_target(state: &State<'_, AppState>, id: &str) -> Result<String, String> {
    if let Some(path) = id.strip_prefix("file:") {
        return Ok(path.replace('/', "\\"));
    }
    let index = state.index.lock().map_err(|e| e.to_string())?;
    index
        .apps
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.target.clone())
        .ok_or_else(|| "找不到该结果，请重新搜索后再试".to_string())
}

/// 当前全部固定项 id（前端用于菜单标签与固定标记）。
#[tauri::command]
pub fn list_pinned(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let hdb = state.history.lock().map_err(|e| e.to_string())?;
    Ok(hdb.pinned_ids())
}

#[tauri::command]
pub fn set_ui_mode(
    settings: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.set_settings_open(settings);
    system::window::set_settings_mode(&app, settings);
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
