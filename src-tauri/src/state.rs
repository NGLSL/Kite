use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

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

/// 重建内存应用索引；可在工作线程调用。
pub fn rebuild_index(app: &AppHandle) -> Result<usize, String> {
    let dir = icon_dir(app);
    let index = crate::app::scan_apps(&dir);
    let count = index.apps.len();
    if let Some(state) = app.try_state::<AppState>() {
        *state.index.lock().map_err(|e| e.to_string())? = index;
    }
    Ok(count)
}
