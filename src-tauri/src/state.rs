use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

/// 两阶段重建：先快速索引可搜索，再后台补图标（补图标时不持锁）。
pub fn rebuild_index(app: &AppHandle) -> Result<usize, String> {
    let t0 = std::time::Instant::now();
    let dir = icon_dir(app);
    crate::log::info(&format!("rebuild start, icon_dir={dir:?}"));

    let index = crate::app::scan_apps(&dir, true);
    let count = index.apps.len();
    if let Some(state) = app.try_state::<AppState>() {
        *state.index.lock().map_err(|e| e.to_string())? = index;
    }
    let _ = app.emit("kite://index-ready", count);
    crate::log::info(&format!(
        "index-ready count={count} in {:?}",
        t0.elapsed()
    ));

    let pending: Vec<(String, Option<String>)> = {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "no state".to_string())?;
        let index = state.index.lock().map_err(|e| e.to_string())?;
        index
            .apps
            .iter()
            .filter(|a| a.icon.is_none())
            .map(|a| (a.id.clone(), Some(a.target.clone())))
            .collect()
    };

    let t1 = std::time::Instant::now();
    let shared: Arc<Mutex<HashMap<String, Option<String>>>> = Arc::new(Mutex::new(HashMap::new()));
    if !pending.is_empty() {
        let chunk = pending.len().div_ceil(8).max(1);
        std::thread::scope(|s| {
            for part in pending.chunks(chunk) {
                let part = part.to_vec();
                let dir = dir.clone();
                let shared = Arc::clone(&shared);
                s.spawn(move || {
                    for (id, src) in &part {
                        let path = crate::system::icons::cache_icon(&dir, id, src.as_deref());
                        if let Ok(mut g) = shared.lock() {
                            g.insert(id.clone(), path);
                        }
                    }
                });
            }
        });
    }

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.index.lock() {
            if let Ok(map) = shared.lock() {
                for item in guard.apps.iter_mut() {
                    if item.icon.is_none() {
                        if let Some(p) = map.get(&item.id) {
                            item.icon = p.clone();
                        }
                    }
                }
            }
            crate::log::info(&format!(
                "icons filled {}/{} in {:?}",
                guard.apps.iter().filter(|a| a.icon.is_some()).count(),
                guard.apps.len(),
                t1.elapsed()
            ));
        }
    }
    let _ = app.emit("kite://icons-ready", count);
    crate::log::info(&format!("rebuild total {:?}", t0.elapsed()));
    Ok(count)
}
