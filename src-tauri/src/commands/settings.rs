//! 设置 / Alias / 历史管理类 IPC。

use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;
use crate::storage::settings::{Settings, UserAlias};
use crate::{search, system};

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let hdb = state.history.lock().map_err(|e| e.to_string())?;
    Ok(hdb.load_settings())
}

#[tauri::command]
pub fn save_settings(
    hide_on_blur: Option<bool>,
    autostart: Option<bool>,
    hotkey: Option<String>,
    search_files: Option<bool>,
    history_recording: Option<bool>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Settings, String> {
    // 锁的持有范围收进块内；热键分支需要换锁，避免跨分支移动 MutexGuard
    let (s, warn, cache_dirty) = {
        let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
        let mut s = hdb.load_settings();
        let mut warn: Option<String> = None;
        let mut cache_dirty = false;

        if let Some(v) = hide_on_blur {
            s.hide_on_blur = v;
            let _ = hdb.save_setting("hide_on_blur", if v { "1" } else { "0" });
            crate::log::info(&format!("save hide_on_blur={v}"));
        }
        if let Some(v) = autostart {
            s.autostart = v;
            let _ = hdb.save_setting("autostart", if v { "1" } else { "0" });
            if let Err(e) = system::autostart::set_autostart(v) {
                // 写库成功但注册表失败：提示但不整体回滚
                warn = Some(format!("开机启动写入失败: {e}"));
                crate::log::info(&warn.clone().unwrap_or_default());
            } else {
                crate::log::info(&format!("save autostart={v}"));
            }
        }
        if let Some(v) = search_files {
            s.search_files = v;
            let _ = hdb.save_setting("search_files", if v { "1" } else { "0" });
            crate::log::info(&format!("save search_files={v}"));
        }
        if let Some(v) = history_recording {
            s.history_recording = v;
            let _ = hdb.save_setting("history_recording", if v { "1" } else { "0" });
            // 历史开关变化后，缓存中的历史加权不再可靠
            cache_dirty = true;
            crate::log::info(&format!("save history_recording={v}"));
        }
        if let Some(h) = hotkey {
            let h = h.trim().to_string();
            if system::hotkey::parse_hotkey(&h).is_none() {
                return Err(format!("无法识别的快捷键: {h}"));
            }
            drop(hdb);
            let label = system::hotkey::reregister(&app, &h)?;
            let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
            let _ = hdb.save_setting("hotkey", &h);
            s.hotkey = h;
            s.hotkey_label = label;
            crate::log::info("save hotkey ok");
        }
        (s, warn, cache_dirty)
    };
    if cache_dirty {
        state.cache_clear();
    }
    let _ = app.emit("kite://settings-changed", ());
    if let Some(w) = warn {
        return Err(w);
    }
    Ok(s)
}

/// 清空使用历史（启动次数 + Query History）。清空后空 Query 不再显示旧记录。
#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.clear_history().map_err(|e| e.to_string())?;
    drop(hdb);
    state.cache_clear();
    Ok(())
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

/// 新增 / 修改用户 Alias。目标必须能解析到索引中的 AppItem：
/// 带稳定 id 直接校验；仅传名称时须精确唯一命中，否则拒绝保存。
#[tauri::command]
pub fn set_user_alias(
    alias: String,
    target_name: String,
    target_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<UserAlias>, String> {
    let tid = target_id.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let (resolved_id, canonical_name) = {
        let index = state.index.lock().map_err(|e| e.to_string())?;
        match tid {
            Some(id) => {
                let item = index
                    .apps
                    .iter()
                    .find(|a| a.id == id)
                    .ok_or_else(|| "目标应用不在索引中，请从候选列表重新选择".to_string())?;
                (Some(id.to_string()), item.display_name.clone())
            }
            None => {
                let qn = search::normalize_for_index(&target_name);
                let exact: Vec<&crate::model::AppItem> = index
                    .apps
                    .iter()
                    .filter(|a| a.normalized_name == qn || a.normalized_display == qn)
                    .collect();
                match exact.as_slice() {
                    [item] => (Some(item.id.clone()), item.display_name.clone()),
                    [] => return Err("目标应用不在索引中，请从候选列表重新选择".to_string()),
                    [_, ..] => return Err("存在多个同名应用，请从候选列表选择具体目标".to_string()),
                }
            }
        }
    };

    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.set_alias(&alias, resolved_id.as_deref(), &canonical_name)
        .map_err(|e| e.to_string())?;
    drop(hdb);
    // Alias 变化立即生效：当前 Query 下次搜索重新排序
    state.cache_clear();
    state
        .history
        .lock()
        .map_err(|e| e.to_string())?
        .list_aliases()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_user_alias(
    alias: String,
    state: State<'_, AppState>,
) -> Result<Vec<UserAlias>, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.remove_alias(&alias).map_err(|e| e.to_string())?;
    drop(hdb);
    state.cache_clear();
    state
        .history
        .lock()
        .map_err(|e| e.to_string())?
        .list_aliases()
        .map_err(|e| e.to_string())
}
