use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use crate::model::AppIndex;
use crate::storage::HistoryDb;

/// Tauri 管理的共享应用状态。
pub struct AppState {
    pub index: Mutex<AppIndex>,
    pub history: Mutex<HistoryDb>,
}

impl AppState {
    pub fn new(history: HistoryDb) -> Self {
        Self {
            index: Mutex::new(AppIndex::empty()),
            history: Mutex::new(history),
        }
    }
}

/// 应用图标缓存目录。
pub fn icon_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_cache_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("icons")
}

/// 历史库路径（应用数据目录）。
pub fn history_db_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("kite-history.db")
}

/// 两阶段重建：
/// 1) 快速建索引（无图标）并立刻可搜索
/// 2) 后台并行补图标后再写回
pub fn rebuild_index(app: &AppHandle) -> Result<usize, String> {
    let dir = icon_dir(app);
    let mut index = crate::app::scan_apps(&dir, true);
    let count = index.apps.len();
    if let Some(state) = app.try_state::<AppState>() {
        *state.index.lock().map_err(|e| e.to_string())? = index;
    }
    let _ = app.emit("kite://index-ready", count);

    // 从共享索引补图标，避免二次全盘扫描
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.index.lock() {
            crate::app::fill_missing_icons(&mut guard, &dir);
        }
    }
    let _ = app.emit("kite://icons-ready", count);
    Ok(count)
}
