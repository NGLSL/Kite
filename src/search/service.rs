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
use crate::search::retrieval::{MatcherScratch, RankedHit, SearchRun};
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

/// 一次搜索的索引来源。
///
/// 首选预建索引：按引用共享，不复制底层数组。只有冷启动尚无预建索引时才退化为
/// 携带后备快照——那条路径是例外，不该成为按键路径的常态开销。
pub enum IndexSource {
    /// 预建索引（`AppIndex::retrieval`）。
    Prebuilt(Arc<RetrievalIndex>),
    /// 后备快照：仅在冷启动无预建索引时使用。
    Snapshot {
        apps: Vec<AppItem>,
        system_entries: Vec<AppItem>,
    },
}

/// 一次应用搜索请求（worker 消费）。
pub struct AppSearchJob {
    pub generation: u64,
    pub index_generation: u64,
    pub query: String,
    pub q_norm: String,
    pub user_targets: Vec<UserTarget>,
    pub prefs: Option<Personalization>,
    pub source: IndexSource,
    pub cache: Arc<BaseHitCache>,
    /// 提交时的缓存失效代际：写回前比对，落后则不写。
    pub cache_epoch: u64,
    /// 完成回调（generation, query, hits, elapsed_us）；取消时不会调用。
    pub on_done: Arc<dyn Fn(u64, String, Vec<SearchResult>, u128) + Send + Sync>,
}

/// 常驻应用搜索 worker：单线程，只跑最新请求。
///
/// 槽位载荷是 `Option<AppSearchJob>`：`Some` 是搜索任务，`None` 是「显式作废」——
/// 清空输入、隐藏窗口这类界面不再需要结果的场景，只推进请求代际让在途任务停下，
/// 不产生新任务，也不 kill 线程。
pub struct AppSearchWorker {
    slot: Arc<LatestSlot<Option<AppSearchJob>>>,
}

impl AppSearchWorker {
    /// 启动唯一 worker 线程。
    pub fn spawn() -> Self {
        Self::spawn_inner(None)
    }

    /// 测试缝：在候选验证开始处回调，用于把「旧任务正在验证」固定成确定的时刻
    /// （而不是靠 sleep 赌调度）。
    #[cfg(test)]
    pub fn spawn_with_verify_hook(hook: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::spawn_inner(Some(hook))
    }

    fn spawn_inner(verify_hook: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let slot = Arc::new(LatestSlot::new());
        let worker_slot = slot.clone();
        std::thread::spawn(move || {
            // 匹配器与字符缓冲常驻：只随线程建一次，之后每个查询只更新 pattern 与解析结果。
            let mut scratch = MatcherScratch::new();
            let mut last_handled = 0u64;
            loop {
                worker_slot.wait_newer_than(last_handled, Duration::from_millis(200));
                let Some((slot_gen, entry)) = worker_slot.take() else {
                    continue;
                };
                last_handled = last_handled.max(slot_gen);
                let Some(job) = entry else {
                    // 显式作废：代际已推进，在途任务会在下一个取消点退出。
                    continue;
                };
                let seq_slot = worker_slot.clone();
                // 协作取消：slot 代际前进即过期（阶段边界与验证批次由检索内核检查）
                let cancelled = move || seq_slot.latest_seq() != slot_gen;
                let run = match verify_hook.as_ref() {
                    Some(hook) => SearchRun::with_verify_hook(&cancelled, hook.as_ref()),
                    None => SearchRun::new(&cancelled),
                };
                run_job(job, &run, &mut scratch);
            }
        });
        Self { slot }
    }

    pub fn submit(&self, job: AppSearchJob) {
        self.slot.submit(Some(job));
    }

    /// 通知 worker 停止当前任务：推进请求失效代际，在途任务在下一个取消点退出。
    /// 不 kill 线程，也不产生新任务。
    pub fn cancel_current(&self) {
        self.slot.submit(None);
    }

    /// 测试缝：当前待处理代际。
    pub fn latest_seq(&self) -> u64 {
        self.slot.latest_seq()
    }
}

fn run_job(job: AppSearchJob, run: &SearchRun<'_>, scratch: &mut MatcherScratch) {
    if run.is_cancelled() {
        return;
    }
    let started = std::time::Instant::now();

    // 后备快照只出现在冷启动；整次任务只建一次索引，阶段 0 与阶段 1 共用。
    let fallback;
    let index = match &job.source {
        IndexSource::Prebuilt(prebuilt) => prebuilt.as_ref(),
        IndexSource::Snapshot {
            apps,
            system_entries,
        } => {
            fallback = RetrievalIndex::build(apps, system_entries);
            &fallback
        }
    };
    // 后备快照的临时索引不写代际缓存：它的 doc_id 只对本次 build 有效。
    let cacheable = matches!(&job.source, IndexSource::Prebuilt(_));

    // 阶段 0：基础候选缓存（个性化前、未截断）
    let base = match job.cache.get(job.index_generation, &job.q_norm) {
        Some(base) => base,
        None => {
            let Some(base) =
                index.search_base_ranked(&job.query, &job.user_targets, scratch, run)
            else {
                return;
            };
            if cacheable && !run.is_cancelled() {
                job.cache.insert_if_epoch(
                    job.cache_epoch,
                    job.index_generation,
                    &job.q_norm,
                    base.clone(),
                );
            }
            base
        }
    };
    if run.is_cancelled() {
        return;
    }

    // 阶段 1：用最新 prefs 完成个性化/归并/排序/截断
    let ranked =
        index.finish_ranked_from_base(base, job.prefs.as_ref(), crate::search::MAX_RESULTS);
    if run.is_cancelled() {
        return;
    }
    let hits = index.materialize_ranked(ranked);
    if run.is_cancelled() {
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
        let mut scratch = MatcherScratch::new();
        let run = SearchRun::new(&|| false);
        let base = index
            .search_base_ranked("chrome", &[], &mut scratch, &run)
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
            source: IndexSource::Snapshot {
                apps: vec![],
                system_entries: vec![],
            },
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

    /// 工作区（matcher + 字符缓冲）跨查询复用时，结果不得依赖「之前查过什么」：
    /// 同一份工作区连跑多个查询，结果与顺序必须与每次新建一致。
    #[test]
    fn reused_scratch_matches_fresh_scratch_across_queries() {
        let apps: Vec<AppItem> = ["Visual Studio Code", "Google Chrome", "微信", "记事本"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mut item = AppItem::scanned(
                    format!("app{i}"),
                    (*name).into(),
                    format!(r"C:\app{i}.exe"),
                    None,
                    None,
                    "test",
                );
                item.attach_search_fields();
                item
            })
            .collect();
        let index = Arc::new(RetrievalIndex::build(&apps, &[]));

        let run_query = |scratch: &mut MatcherScratch, query: &str| -> Vec<(String, i32)> {
            let captured = Arc::new(Mutex::new(Vec::<(String, i32)>::new()));
            let sink = captured.clone();
            let job = AppSearchJob {
                generation: 1,
                index_generation: 1,
                query: query.into(),
                q_norm: query.into(),
                user_targets: vec![],
                prefs: None,
                source: IndexSource::Prebuilt(index.clone()),
                cache: Arc::new(BaseHitCache::new(4)),
                cache_epoch: 0,
                on_done: Arc::new(move |_, _, hits, _| {
                    let mut guard = sink.lock().unwrap_or_else(|e| e.into_inner());
                    *guard = hits.into_iter().map(|h| (h.item.id, h.score)).collect();
                }),
            };
            run_job(job, &SearchRun::new(&|| false), scratch);
            let out = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
            out
        };

        let mut shared = MatcherScratch::new();
        for query in ["code", "微信", "chrome", "jsb", "code"] {
            let reused = run_query(&mut shared, query);
            let fresh = run_query(&mut MatcherScratch::new(), query);
            assert_eq!(
                reused, fresh,
                "复用工作区后 query={query:?} 的结果与顺序必须与新建一致"
            );
        }
    }

    /// 两条索引来源路径必须等价：有预建索引时按引用共享，冷启动无预建索引时
    /// 携带后备快照。结果与顺序一致才算收敛成功。
    #[test]
    fn prebuilt_and_snapshot_sources_agree() {
        let apps: Vec<AppItem> = ["Visual Studio Code", "Google Chrome", "微信", "记事本"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mut item = AppItem::scanned(
                    format!("app{i}"),
                    (*name).into(),
                    format!(r"C:\app{i}.exe"),
                    None,
                    None,
                    "test",
                );
                item.attach_search_fields();
                item
            })
            .collect();
        let prebuilt = Arc::new(RetrievalIndex::build(&apps, &[]));

        let run_query = |source: IndexSource| -> Vec<String> {
            let captured = Arc::new(Mutex::new(Vec::<String>::new()));
            let sink = captured.clone();
            let job = AppSearchJob {
                generation: 1,
                index_generation: 1,
                query: "code".into(),
                q_norm: "code".into(),
                user_targets: vec![],
                prefs: None,
                source,
                cache: Arc::new(BaseHitCache::new(4)),
                cache_epoch: 0,
                on_done: Arc::new(move |_, _, hits, _| {
                    let mut guard = sink.lock().unwrap_or_else(|e| e.into_inner());
                    *guard = hits.into_iter().map(|h| h.item.id).collect();
                }),
            };
            let mut scratch = MatcherScratch::new();
            run_job(job, &SearchRun::new(&|| false), &mut scratch);
            let out = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
            out
        };

        let shared = run_query(IndexSource::Prebuilt(prebuilt));
        let snapshot = run_query(IndexSource::Snapshot {
            apps: apps.clone(),
            system_entries: vec![],
        });

        assert!(!shared.is_empty(), "预建索引路径应有命中");
        assert_eq!(
            shared, snapshot,
            "预建索引与后备快照必须给出同样的结果与顺序"
        );
    }

    /// 用测试屏障把「旧任务刚进入候选验证、还没产出任何东西」固定成确定的时刻，
    /// 再提交新任务：断言旧任务被取消——不回调、不写缓存，新任务照常完成。
    /// 语义与「计算中清空查询」同一条：两者都只推进请求失效代际。
    /// （「按批次而非只在进循环前查取消」由 `verify::verify_all` 的单测固定。）
    #[test]
    fn cancel_during_verify_skips_callback_and_cache_write() {
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release = Arc::new(Mutex::new(release_rx));
        // 只钉住第一次进入验证的任务；后续任务不阻塞。
        let first_only = Arc::new(AtomicBool::new(false));
        let hook = {
            let release = release.clone();
            let first_only = first_only.clone();
            Arc::new(move || {
                if first_only.swap(true, Ordering::SeqCst) {
                    return;
                }
                let _ = entered_tx.send(());
                let guard = release.lock().unwrap_or_else(|e| e.into_inner());
                let _ = guard.recv();
            }) as Arc<dyn Fn() + Send + Sync>
        };
        let worker = AppSearchWorker::spawn_with_verify_hook(hook);
        let cache = Arc::new(BaseHitCache::new(8));
        let (tx, rx) = mpsc::channel::<u64>();

        let mut item = AppItem::scanned(
            "vscode".into(),
            "Visual Studio Code".into(),
            r"C:\vscode.exe".into(),
            None,
            None,
            "test",
        );
        item.attach_search_fields();
        let index = Arc::new(RetrievalIndex::build(&[item], &[]));

        let job = |generation: u64, source: IndexSource, tx: mpsc::Sender<u64>| AppSearchJob {
            generation,
            index_generation: 1,
            query: "code".into(),
            q_norm: "code".into(),
            user_targets: vec![],
            prefs: None,
            source,
            cache: cache.clone(),
            cache_epoch: cache.epoch(),
            on_done: Arc::new(move |generation, _, _, _| {
                let _ = tx.send(generation);
            }),
        };

        // 旧任务：真实索引 + 可召回查询，会进入候选验证
        worker.submit(job(1, IndexSource::Prebuilt(index), tx.clone()));
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("旧任务应进入候选验证");

        // 旧任务被钉在验证里时：清空输入 + 再次提交同一查询
        worker.cancel_current();
        worker.submit(job(
            2,
            IndexSource::Snapshot {
                apps: vec![],
                system_entries: vec![],
            },
            tx,
        ));
        let _ = release_tx.send(());

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2))
                .expect("最新请求应完成"),
            2,
            "旧任务不得回调——它的结果已经作废"
        );
        assert!(
            cache.get(1, "code").is_none(),
            "被取消的任务不得把候选写回缓存"
        );
    }
}
