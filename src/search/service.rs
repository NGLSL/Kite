//! 常驻搜索调度：小容量基础命中缓存 + 个性化重放辅助。
//!
//! 职责边界：
//! - 缓存：仅无个性化时缓存截断前统一排序结果；有历史/Pin/降权则由调用方重算
//! - UI 负责 Query 代际与过期丢弃（与文件查询同一模式）

use std::collections::HashMap;
use std::sync::Mutex;

use crate::history::Personalization;
use crate::model::SearchResult;

/// 小容量基础命中缓存：key = (index_gen, query_norm, max_results)。
#[derive(Debug)]
pub struct BaseHitCache {
    map: Mutex<HashMap<(u64, String, usize), Vec<SearchResult>>>,
    capacity: usize,
}

impl Default for BaseHitCache {
    fn default() -> Self {
        Self::new(32)
    }
}

impl BaseHitCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            capacity: capacity.max(1),
        }
    }

    pub fn get(
        &self,
        index_gen: u64,
        query_norm: &str,
        max_results: usize,
    ) -> Option<Vec<SearchResult>> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&(index_gen, query_norm.to_string(), max_results))
            .cloned()
    }

    pub fn insert(
        &self,
        index_gen: u64,
        query_norm: &str,
        max_results: usize,
        hits: Vec<SearchResult>,
    ) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.capacity {
            map.clear();
        }
        map.insert((index_gen, query_norm.to_string(), max_results), hits);
    }

    pub fn clear(&self) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn len(&self) -> usize {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 在基础命中上重放最新个性化（历史/Pin/降权）。
pub fn personalize_base_hits(
    base: Vec<SearchResult>,
    prefs: Option<&Personalization>,
) -> Vec<SearchResult> {
    let Some(prefs) = prefs else {
        return base;
    };
    let mut hits = base;
    crate::history::apply_personalization(&mut hits, prefs);
    hits
}

/// 个性化快照是否为空（可安全缓存基础命中）。
pub fn prefs_is_empty(prefs: Option<&Personalization>) -> bool {
    prefs
        .map(|p| {
            p.usage.is_empty()
                && p.pairs.is_empty()
                && p.pinned.is_empty()
                && p.demoted.is_empty()
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppItem;

    fn hit(id: &str, score: i32) -> SearchResult {
        SearchResult::scored(
            AppItem::scanned(
                id.into(),
                id.into(),
                format!(r"C:\{id}.exe"),
                None,
                None,
                "test",
            ),
            score,
            "base",
        )
    }

    #[test]
    fn cache_roundtrip_same_query() {
        let cache = BaseHitCache::new(4);
        let hits = vec![hit("a", 100), hit("b", 80)];
        cache.insert(1, "chrome", 10, hits);
        let got = cache.get(1, "chrome", 10).expect("cache hit");
        assert_eq!(got.len(), 2);
        assert!(cache.get(2, "chrome", 10).is_none());
    }

    #[test]
    fn cache_capacity_clears_old_entries() {
        let cache = BaseHitCache::new(2);
        cache.insert(1, "a", 10, vec![hit("a", 1)]);
        cache.insert(1, "b", 10, vec![hit("b", 1)]);
        cache.insert(1, "c", 10, vec![hit("c", 1)]);
        assert_eq!(cache.len(), 1);
        assert!(cache.get(1, "c", 10).is_some());
    }

    #[test]
    fn empty_prefs_allows_cache() {
        assert!(prefs_is_empty(None));
        assert!(prefs_is_empty(Some(&Personalization::default())));
        let mut p = Personalization::default();
        p.demoted.insert("x".into());
        assert!(!prefs_is_empty(Some(&p)));
    }

    #[test]
    fn personalization_recomputed_on_cached_base() {
        let base = vec![hit("used", 500), hit("other", 500)];
        let mut prefs = Personalization::default();
        prefs.query_norm = "q".into();
        prefs.usage.insert(
            "used".into(),
            crate::storage::UsageStats {
                launch_count: 10,
                last_used_at: 1_700_000_000,
            },
        );
        prefs.now = 1_700_000_000;
        let ranked = personalize_base_hits(base, Some(&prefs));
        assert_eq!(ranked[0].item.id, "used");
    }

    #[test]
    fn cache_clear_on_invalidate() {
        let cache = BaseHitCache::new(4);
        cache.insert(1, "x", 10, vec![hit("x", 1)]);
        cache.clear();
        assert!(cache.get(1, "x", 10).is_none());
    }
}
