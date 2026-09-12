use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use crate::model::{AppIndex, SearchResult};
use crate::storage::HistoryDb;

/// Tauri 管理的共享应用状态。
pub struct AppState {
    pub index: Mutex<AppIndex>,
    pub history: Mutex<HistoryDb>,
    /// 设置页打开时禁止失焦隐藏，避免缩放/点控件误收起。
    pub settings_open: AtomicBool,
    /// 滚动加载：最近一次查询的完整排序结果（键 = Query + 是否含文件）。
    /// 只保留最近一条；滚动放大条数时直接切片，不重搜、不重复查 Everything。
    pub search_cache: Mutex<Option<SearchCache>>,
}

/// 一条查询的完整排序结果缓存。
#[derive(Debug, Clone)]
pub struct SearchCache {
    pub query_norm: String,
    pub include_files: bool,
    pub ranked: Vec<SearchResult>,
}

impl AppState {
    pub fn new(history: HistoryDb) -> Self {
        Self {
            index: Mutex::new(AppIndex::empty()),
            history: Mutex::new(history),
            settings_open: AtomicBool::new(false),
            search_cache: Mutex::new(None),
        }
    }

    pub fn is_settings_open(&self) -> bool {
        self.settings_open.load(Ordering::Relaxed)
    }

    pub fn set_settings_open(&self, open: bool) {
        self.settings_open.store(open, Ordering::Relaxed);
    }

    /// 同 Query 的前 `limit` 条；查询不同则 None。
    pub fn cache_get(
        &self,
        query_norm: &str,
        include_files: bool,
        limit: usize,
    ) -> Option<Vec<SearchResult>> {
        let guard = self.search_cache.lock().ok()?;
        let cache = guard.as_ref()?;
        if cache.query_norm != query_norm || cache.include_files != include_files {
            return None;
        }
        let end = limit.min(cache.ranked.len());
        Some(cache.ranked[..end].to_vec())
    }

    pub fn cache_put(&self, query_norm: String, include_files: bool, ranked: Vec<SearchResult>) {
        if let Ok(mut guard) = self.search_cache.lock() {
            *guard = Some(SearchCache {
                query_norm,
                include_files,
                ranked,
            });
        }
    }

    pub fn cache_clear(&self) {
        if let Ok(mut guard) = self.search_cache.lock() {
            *guard = None;
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
    // 图标缓存按版本失效：提取算法变更时递增版本号才清缓存，否则跨启动复用，
    // 避免每次启动都全量重提图标（数百次 GDI 提取 + 磁盘写）。
    const ICON_CACHE_VERSION: &str = "1";
    let marker = dir.join("cache-ver");
    let valid = std::fs::read_to_string(&marker)
        .map(|v| v.trim() == ICON_CACHE_VERSION)
        .unwrap_or(false);
    if !valid {
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&marker, ICON_CACHE_VERSION);
        crate::log::info(&format!(
            "rebuild start, icon_dir={dir:?} cache cleared (version {ICON_CACHE_VERSION})"
        ));
    } else {
        crate::log::info(&format!("rebuild start, icon_dir={dir:?} cache reused"));
    }

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
        state.cache_clear();
    }
    let _ = app.emit("kite://index-ready", count);
    crate::log::info(&format!(
        "index-ready count={count} in {:?}",
        t0.elapsed()
    ));

    // 图标补齐三段式：短暂持锁收集 → 无锁并行提取 → 短暂持锁合并
    let t1 = std::time::Instant::now();
    let pending: Vec<(String, Option<String>)> = {
        let state = app.try_state::<AppState>().ok_or("no state")?;
        let index = state.index.lock().map_err(|e| e.to_string())?;
        crate::app::scanner::missing_icon_targets(&index)
    };
    let filled = crate::app::scanner::extract_icons_parallel(&pending, &dir);
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut guard) = state.index.lock() {
            for item in guard.apps.iter_mut() {
                if item.icon.is_none() {
                    if let Some(p) = filled.get(&item.id) {
                        item.icon = p.clone();
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
                state.cache_clear();
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

/// 把 UWP 条目去重后并入现有索引；图标提取全程不持索引锁，避免卡住搜索。
fn merge_uwp_apps(
    app: &AppHandle,
    raw: Vec<(crate::model::AppItem, Option<String>)>,
    icon_dir: &std::path::Path,
) -> usize {
    use crate::app::scanner::util::normalize_path_key;
    let Some(state) = app.try_state::<AppState>() else {
        return 0;
    };

    // 1) 短暂持锁：读取现有 key
    let mut known: std::collections::HashSet<String> = {
        let Ok(guard) = state.index.lock() else {
            return 0;
        };
        guard
            .apps
            .iter()
            .map(|a| normalize_path_key(&a.target))
            .collect()
    };

    // 2) 不持锁：去重、补字段、提图标
    let mut to_add: Vec<crate::model::AppItem> = Vec::new();
    for (mut item, icon_src) in raw {
        let key = normalize_path_key(&item.target);
        if !known.insert(key) {
            continue;
        }
        item.attach_search_fields();
        item.icon_src = icon_src.or_else(|| Some(item.target.clone()));
        item.icon =
            crate::system::icons::cache_icon(icon_dir, &item.id, item.icon_src.as_deref());
        to_add.push(item);
    }

    // 3) 短暂持锁：合并（再按 key 去重，防并发窗口内索引已更新）
    let mut added = 0usize;
    if let Ok(mut guard) = state.index.lock() {
        let existing: std::collections::HashSet<String> = guard
            .apps
            .iter()
            .map(|a| normalize_path_key(&a.target))
            .collect();
        for item in to_add {
            if existing.contains(&normalize_path_key(&item.target)) {
                continue;
            }
            guard.apps.push(item);
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppItem;

    fn test_state() -> AppState {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kite-state-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = HistoryDb::open(&dir.join("state-test.db")).expect("temp db");
        AppState::new(db)
    }

    fn hit(id: &str) -> SearchResult {
        SearchResult {
            item: AppItem::scanned(id.into(), id.into(), format!("C:\\{id}.exe"), None, None, "t"),
            score: 1,
            matched_by: "test".into(),
        }
    }

    #[test]
    fn search_cache_slice_by_limit() {
        let state = test_state();
        assert!(state.cache_get("ch", false, 5).is_none(), "未缓存时 None");
        let ranked: Vec<_> = (0..20).map(|i| hit(&format!("app{i:02}"))).collect();
        state.cache_put("ch".into(), false, ranked);

        assert_eq!(state.cache_get("ch", false, 5).unwrap().len(), 5);
        assert_eq!(state.cache_get("ch", false, 50).unwrap().len(), 20);
        // 键不同(查询或文件开关变化)必须未命中,走完整搜索
        assert!(state.cache_get("ch", true, 5).is_none());
        assert!(state.cache_get("chr", false, 5).is_none());
    }

    #[test]
    fn search_cache_clear_on_rebuild_signal() {
        let state = test_state();
        state.cache_put("q".into(), false, vec![hit("a")]);
        state.cache_clear();
        assert!(state.cache_get("q", false, 1).is_none());
    }

    #[test]
    fn search_cache_keeps_latest_query_only() {
        let state = test_state();
        state.cache_put("a".into(), false, vec![hit("a1")]);
        state.cache_put("b".into(), false, vec![hit("b1"), hit("b2")]);
        assert!(state.cache_get("a", false, 1).is_none());
        assert_eq!(state.cache_get("b", false, 2).unwrap().len(), 2);
    }
}
