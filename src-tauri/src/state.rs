use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};

use crate::model::AppIndex;
use crate::storage::HistoryDb;

/// Tauri 管理的共享应用状态。
pub struct AppState {
    pub index: Mutex<AppIndex>,
    pub history: Mutex<HistoryDb>,
    /// 设置页打开时禁止失焦隐藏，避免缩放/点控件误收起。
    pub settings_open: AtomicBool,
}

impl AppState {
    pub fn new(history: HistoryDb) -> Self {
        Self {
            index: Mutex::new(AppIndex::empty()),
            history: Mutex::new(history),
            settings_open: AtomicBool::new(false),
        }
    }

    pub fn is_settings_open(&self) -> bool {
        self.settings_open.load(Ordering::Relaxed)
    }

    pub fn set_settings_open(&self, open: bool) {
        self.settings_open.store(open, Ordering::Relaxed);
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
    // 旧缓存可能是未裁边的小图标，首次重建时清掉
    let _ = std::fs::remove_dir_all(&dir);
    crate::log::info(&format!("rebuild start, icon_dir={dir:?} cleared_cache"));

    // 扫描可能因系统异常 panic；兜底为空索引，避免线程静默死亡
    let index = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::app::scan_apps(&dir, true)
    })) {
        Ok(idx) => idx,
        Err(_) => {
            crate::log::info("scan_apps panicked; fallback empty index");
            crate::model::AppIndex::empty()
        }
    };
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
            .map(|a| {
                let src = a
                    .icon_src
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| Some(a.target.clone()));
                (a.id.clone(), src)
            })
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

    // 首屏不堵：后台补扫 UWP / Store 应用，合并进索引
    let handle = app.clone();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let mut raw = Vec::new();
        crate::app::uwp::collect_uwp("uwp", &mut raw);
        if raw.is_empty() {
            crate::log::info("uwp background: empty");
            return;
        }
        let total_scan = raw.len();
        let added = merge_uwp_apps(&handle, raw, &dir);
        crate::log::info(&format!(
            "uwp background: +{added} total_scan={total_scan} in {:?}",
            t.elapsed()
        ));
        if added > 0 {
            if let Some(state) = handle.try_state::<AppState>() {
                if let Ok(idx) = state.index.lock() {
                    let n = idx.apps.len();
                    let _ = handle.emit("kite://index-ready", n);
                    let _ = handle.emit("kite://icons-ready", n);
                }
            }
        }
    });

    Ok(count)
}

/// 把 UWP 条目去重后并入现有索引，并尝试提图标。
fn merge_uwp_apps(
    app: &AppHandle,
    raw: Vec<(crate::model::AppItem, Option<String>)>,
    icon_dir: &std::path::Path,
) -> usize {
    use crate::app::scanner::util::normalize_path_key;
    let Some(state) = app.try_state::<AppState>() else {
        return 0;
    };
    let Ok(mut guard) = state.index.lock() else {
        return 0;
    };
    let mut known: std::collections::HashSet<String> = guard
        .apps
        .iter()
        .map(|a| normalize_path_key(&a.target))
        .collect();
    let mut added = 0usize;
    for (mut item, icon_src) in raw {
        let key = normalize_path_key(&item.target);
        if known.contains(&key) {
            continue;
        }
        known.insert(key);
        item.attach_search_fields();
        item.icon_src = icon_src.or_else(|| Some(item.target.clone()));
        item.icon = crate::system::icons::cache_icon(
            icon_dir,
            &item.id,
            item.icon_src.as_deref(),
        );
        guard.apps.push(item);
        added += 1;
    }
    added
}
