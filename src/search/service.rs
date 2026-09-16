//! 常驻搜索调度：最新请求槽 + 单 worker + 协作取消 + 基础候选缓存。
//!
//! 职责边界：
//! - 待处理请求只保留最新一份（覆盖，不排队）
//! - 单一常驻线程；旧任务在阶段边界协作退出
//! - 缓存的是**个性化前**的轻量候选（未截断），个性化每次用最新 prefs 重放

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::history::Personalization;
use crate::model::{AppItem, SearchResult};
use crate::search::matcher::UserTarget;
use crate::search::retrieval::RankedHit;
use crate::search::RetrievalIndex;

/// 基础轻量候选缓存：key = (index_gen, query_norm)。
/// value 为 into_ranked 后、个性化/归并/截断前的候选（不是 TopN 全集）。
///
/// 代际（`epoch`）解决的是「只清空挡不住在途任务」：别名一类基础候选依赖变化时，
/// 清空 map 还不够——正在跑的那次搜索完成时会把用旧别名算出的候选写回来，
/// 缓存随即自我污染。写入必须携带提交时的代际，代际落后即丢弃。
#[derive(Debug)]
pub struct BaseHitCache {
    map: Mutex<HashMap<(u64, String), Vec<RankedHit>>>,
    capacity: usize,
    epoch: AtomicU64,
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
            epoch: AtomicU64::new(0),
        }
    }

    pub fn get(&self, index_gen: u64, query_norm: &str) -> Option<Vec<RankedHit>> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&(index_gen, query_norm.to_string())).cloned()
    }

    /// 当前失效代际；提交任务时读取，写回时比对。
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// 仅在缓存代际未变时写入，返回是否真正写入。
    /// 与 [`BaseHitCache::invalidate`] 共用同一把锁，代际判断在锁内完成。
    pub fn insert_if_epoch(
        &self,
        epoch: u64,
        index_gen: u64,
        query_norm: &str,
        base: Vec<RankedHit>,
    ) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if self.epoch.load(Ordering::SeqCst) != epoch {
            return false;
        }
        if map.len() >= self.capacity {
            map.clear();
        }
        map.insert((index_gen, query_norm.to_string()), base);
        true
    }

    /// 清空并推进代际：既让当前缓存作废，也让在途任务的结果无法再写回。
    pub fn invalidate(&self) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.clear();
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// 仅清空，不推进代际。用于索引重建：缓存键已含索引代际，旧键不会再被命中。
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

/// 在 SearchResult 基础命中上重放个性化（测试/工具路径）。
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
    /// 提交时的缓存失效代际：写回前比对，落后则不写。
    pub cache_epoch: u64,
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

    // 阶段 0：基础候选缓存（个性化前、未截断）
    let base = if let Some(base) = job.cache.get(job.index_generation, &job.q_norm) {
        Some(base)
    } else if let Some(ret) = job.retrieval.as_deref() {
        match ret.search_base_ranked(&job.query, &job.user_targets, cancelled) {
            Some(base) => {
                if !cancelled() {
                    job.cache.insert_if_epoch(
                        job.cache_epoch,
                        job.index_generation,
                        &job.q_norm,
                        base.clone(),
                    );
                }
                Some(base)
            }
            None => return,
        }
    } else {
        if cancelled() {
            return;
        }
        let index = RetrievalIndex::build(&job.apps, &job.system_entries);
        // 临时索引不跨请求复用，不写入代际缓存（doc_id 无效于下一次 build）
        index.search_base_ranked(&job.query, &job.user_targets, cancelled)
    };

    let Some(base) = base else {
        return;
    };
    if cancelled() {
        return;
    }

    // 阶段 1：用最新 prefs 完成个性化/归并/排序/截断
    let hits = if let Some(ret) = job.retrieval.as_deref() {
        let ranked = ret.finish_ranked_from_base(base, job.prefs.as_ref(), crate::search::MAX_RESULTS);
        if cancelled() {
            return;
        }
        ret.materialize_ranked(ranked)
    } else {
        if cancelled() {
            return;
        }
        let index = RetrievalIndex::build(&job.apps, &job.system_entries);
        let ranked = index.finish_ranked_from_base(base, job.prefs.as_ref(), crate::search::MAX_RESULTS);
        if cancelled() {
            return;
        }
        index.materialize_ranked(ranked)
    };

    if cancelled() {
        return;
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
        let mut it = AppItem::scanned(
            "a".into(),
            "chrome".into(),
            r"C:\a.exe".into(),
            None,
            None,
            "t",
        );
        it.attach_search_fields();
        let index = RetrievalIndex::build(&[it], &[]);
        let base = index
            .search_base_ranked("chrome", &[], &|| false)
            .expect("base");
        assert!(cache.insert_if_epoch(cache.epoch(), 1, "chrome", base));
        assert!(cache.get(1, "chrome").is_some());
        assert!(cache.get(2, "chrome").is_none());
    }

    #[test]
    fn cache_capacity_clears_old_entries() {
        let cache = BaseHitCache::new(2);
        let epoch = cache.epoch();
        cache.insert_if_epoch(epoch, 1, "a", vec![]);
        cache.insert_if_epoch(epoch, 1, "b", vec![]);
        cache.insert_if_epoch(epoch, 1, "c", vec![]);
        assert_eq!(cache.len(), 1);
        assert!(cache.get(1, "c").is_some());
    }

    #[test]
    fn invalidate_rejects_writeback_from_in_flight_search() {
        let cache = BaseHitCache::new(4);
        // 别名这类基础候选依赖变化：清空 + 推进代际
        let epoch = cache.epoch();
        cache.invalidate();

        assert!(
            !cache.insert_if_epoch(epoch, 1, "qa", vec![]),
            "失效前提交的在途任务不得把旧别名结果写回"
        );
        assert!(cache.get(1, "qa").is_none());
        assert!(cache.insert_if_epoch(cache.epoch(), 1, "qa", vec![]));
        assert!(cache.get(1, "qa").is_some());
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
        cache.insert_if_epoch(cache.epoch(), 1, "x", vec![]);
        cache.clear();
        assert!(cache.get(1, "x").is_none());
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
            cache_epoch: cache.epoch(),
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
