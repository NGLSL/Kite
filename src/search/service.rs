//! 常驻搜索调度：最新请求槽 + 单 worker + 协作取消 + 基础命中缓存。
//!
//! 职责边界：
//! - 待处理请求只保留最新一份（覆盖，不排队）
//! - 单一常驻线程；旧任务在阶段边界协作退出
//! - 缓存：仅无个性化且任务完整完成时写入

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::history::Personalization;
use crate::model::{AppItem, SearchResult};
use crate::search::matcher::UserTarget;
use crate::search::RetrievalIndex;

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

/// 最新请求槽：只保留一份，submit 覆盖旧值。
#[derive(Debug)]
pub struct LatestSlot<T> {
    inner: Mutex<SlotState<T>>,
    seq: AtomicU64,
    wake: Arc<(Mutex<()>, Condvar)>,
}

#[derive(Debug)]
struct SlotState<T> {
    /// (提交代际, 任务)
    pending: Option<(u64, T)>,
}

impl<T> Default for LatestSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LatestSlot<T> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SlotState { pending: None }),
            seq: AtomicU64::new(0),
            wake: Arc::new((Mutex::new(()), Condvar::new())),
        }
    }

    /// 覆盖写入并返回本次代际。
    pub fn submit(&self, value: T) -> u64 {
        let gen = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.pending = Some((gen, value));
        }
        let (lock, cv) = &*self.wake;
        let _g = lock.lock().unwrap_or_else(|e| e.into_inner());
        cv.notify_all();
        gen
    }

    pub fn latest_seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }

    /// 取出当前待处理项及其提交代际。
    pub fn take(&self) -> Option<(u64, T)> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.pending.take()
    }

    /// 阻塞直到出现比 `seen` 更新的提交，或超时。
    pub fn wait_newer_than(&self, seen: u64, timeout: Duration) -> bool {
        let (lock, cv) = &*self.wake;
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = guard;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.latest_seq() > seen {
                return true;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return self.latest_seq() > seen;
            }
            let (g, _t) = cv
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            guard = g;
        }
    }
}

/// 一次应用搜索请求（worker 消费）。
pub struct AppSearchJob {
    pub generation: u64,
    pub index_generation: u64,
    pub query: String,
    pub q_norm: String,
    pub user_targets: Vec<UserTarget>,
    pub prefs: Option<Personalization>,
    pub retrieval: Option<Arc<RetrievalIndex>>,
    pub apps: Vec<AppItem>,
    pub system_entries: Vec<AppItem>,
    pub cache: Arc<BaseHitCache>,
    /// 完成回调（generation, query, hits, elapsed_us）；取消时不会调用。
    pub on_done: Arc<dyn Fn(u64, String, Vec<SearchResult>, u128) + Send + Sync>,
}

/// 常驻应用搜索 worker：单线程，只跑最新请求。
pub struct AppSearchWorker {
    slot: Arc<LatestSlot<AppSearchJob>>,
}

impl AppSearchWorker {
    /// 启动唯一 worker 线程。
    pub fn spawn() -> Self {
        let slot = Arc::new(LatestSlot::new());
        let worker_slot = slot.clone();
        std::thread::spawn(move || {
            let mut last_handled = 0u64;
            loop {
                worker_slot.wait_newer_than(last_handled, Duration::from_millis(200));
                let Some((slot_gen, job)) = worker_slot.take() else {
                    continue;
                };
                last_handled = last_handled.max(slot_gen);
                let seq_slot = worker_slot.clone();
                // 协作取消：slot 代际前进即过期（阶段边界由内核检查）
                let cancelled = move || seq_slot.latest_seq() != slot_gen;
                run_job(job, &cancelled);
            }
        });
        Self { slot }
    }

    pub fn submit(&self, job: AppSearchJob) {
        self.slot.submit(job);
    }

    /// 测试缝：当前待处理代际。
    pub fn latest_seq(&self) -> u64 {
        self.slot.latest_seq()
    }
}

fn run_job(job: AppSearchJob, cancelled: &dyn Fn() -> bool) {
    if cancelled() {
        return;
    }
    let started = std::time::Instant::now();

    // 阶段 0：缓存命中（仅无个性化）
    if prefs_is_empty(job.prefs.as_ref()) {
        if let Some(hits) = job.cache.get(
            job.index_generation,
            &job.q_norm,
            crate::search::MAX_RESULTS,
        ) {
            if !cancelled() {
                (job.on_done)(job.generation, job.query, hits, started.elapsed().as_micros());
            }
            return;
        }
    }

    if cancelled() {
        return;
    }

    // 阶段 1–3：召回 / 验证 / 排序（内核在阶段边界检查 cancelled）
    let hits = if let Some(ret) = job.retrieval.as_deref() {
        ret.search_personalized_cancellable(
            &job.query,
            &job.user_targets,
            job.prefs.as_ref(),
            crate::search::MAX_RESULTS,
            cancelled,
        )
    } else {
        // 无预构建索引时一次性构建；构建前再查一次取消
        if cancelled() {
            return;
        }
        let index = RetrievalIndex::build(&job.apps, &job.system_entries);
        index.search_personalized_cancellable(
            &job.query,
            &job.user_targets,
            job.prefs.as_ref(),
            crate::search::MAX_RESULTS,
            cancelled,
        )
    };

    let Some(hits) = hits else {
        // 取消：无部分结果、不写缓存、不回调
        return;
    };
    if cancelled() {
        return;
    }

    // 仅无个性化且完整完成才写缓存
    if prefs_is_empty(job.prefs.as_ref()) {
        job.cache.insert(
            job.index_generation,
            &job.q_norm,
            crate::search::MAX_RESULTS,
            hits.clone(),
        );
    }

    let elapsed = started.elapsed().as_micros();
    (job.on_done)(job.generation, job.query, hits, elapsed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;

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

    #[test]
    fn latest_slot_overwrites_pending() {
        let slot: LatestSlot<u32> = LatestSlot::new();
        let g1 = slot.submit(1);
        let g2 = slot.submit(2);
        assert!(g2 > g1);
        let taken = slot.take();
        assert_eq!(taken.map(|(g, v)| (g, v)), Some((g2, 2)));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn wait_wakes_on_newer_submit() {
        let slot: LatestSlot<u32> = LatestSlot::new();
        let s = Arc::new(slot);
        let s2 = s.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            s2.submit(9);
        });
        assert!(s.wait_newer_than(0, Duration::from_millis(500)));
        let taken = s.take();
        assert_eq!(taken.map(|(_, v)| v), Some(9));
    }

    #[test]
    fn cancellable_search_returns_none_when_cancelled() {
        let apps = vec![
            AppItem::scanned(
                "a".into(),
                "Visual Code".into(),
                r"C:\a.exe".into(),
                None,
                None,
                "test",
            ),
        ];
        let mut item = apps[0].clone();
        item.attach_search_fields();
        let index = RetrievalIndex::build(&[item], &[]);
        let flag = AtomicBool::new(false);
        let cancelled = || flag.load(Ordering::SeqCst);
        // 先不取消：有结果
        let ok = index.search_personalized_cancellable("code", &[], None, 10, &cancelled);
        assert!(ok.unwrap().len() >= 1);
        flag.store(true, Ordering::SeqCst);
        let cancelled_hit = index.search_personalized_cancellable("code", &[], None, 10, &cancelled);
        assert!(cancelled_hit.is_none(), "取消后不得返回部分结果");
    }

    #[test]
    fn worker_latest_wins_and_skips_stale_callback() {
        let (tx, rx) = mpsc::channel::<u64>();
        let worker = AppSearchWorker::spawn();
        let cache = Arc::new(BaseHitCache::new(8));

        let make_job = |gen: u64, q: &str, tx: mpsc::Sender<u64>| AppSearchJob {
            generation: gen,
            index_generation: 0,
            query: q.to_string(),
            q_norm: q.to_string(),
            user_targets: vec![],
            prefs: None,
            retrieval: None,
            apps: vec![],
            system_entries: vec![],
            cache: cache.clone(),
            on_done: Arc::new(move |generation, _, _, _| {
                let _ = tx.send(generation);
            }),
        };

        // 慢任务：空索引也走完整构建路径；连续 submit 覆盖
        worker.submit(make_job(1, "aaa", tx.clone()));
        worker.submit(make_job(2, "bbb", tx.clone()));
        worker.submit(make_job(3, "ccc", tx));

        // 最终应只看到最新（或至多最新一档）；不应出现旧 gen 完成回调
        let mut seen = Vec::new();
        while let Ok(g) = rx.recv_timeout(Duration::from_millis(800)) {
            seen.push(g);
        }
        assert!(
            seen.iter().all(|&g| g >= 2),
            "不应回调更旧代际: {seen:?}"
        );
        assert!(
            seen.contains(&3),
            "最新请求应完成: {seen:?}"
        );
    }
}
