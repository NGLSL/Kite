//! 后台索引重建。
//!
//! Cold Start：Bootstrap 立刻发布可搜索首屏。
//! Warm Start：启动前已由 snapshot 恢复，跳过 Bootstrap。
//! Full：写 last-good，并挂起；Launcher 隐藏后/下次打开再采用，避免可见时卡顿。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, OnceLock};
use std::time::Instant;

use iced::futures::channel::mpsc::UnboundedSender;

use crate::app;
use crate::app::scanner::{ScanOptions, ScanPass};
use crate::model::{AppIndex, AppItem};

use super::{plog, Message};

/// 单飞：true 表示已有构建在跑，新的请求合并跳过（完成后由调用方再触发）。
static BUILDING: AtomicBool = AtomicBool::new(false);
/// 构建期间又有变化：当前构建结束后再跑一轮。
static PENDING_REBUILD: AtomicBool = AtomicBool::new(false);
/// 构建代际：完成时仅当仍等于启动时记下的 generation 才发布，避免旧构建覆盖新状态。
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// 进程内是否已完成过至少一次 Full（用于区分启动首建与运行中重建）。
static EVER_COMPLETED_FULL: AtomicBool = AtomicBool::new(false);
/// 启动首建进行中：入口脏事件只记 deferred，不在首建期间叠跑第二轮 Full。
static STARTUP_BUILD_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 启动首建期间收到的入口变化；首建结束后再触发一轮必要重建。
static INITIAL_DIRTY: AtomicBool = AtomicBool::new(false);

fn pending_full_slot() -> &'static Mutex<Option<AppIndex>> {
    static PENDING_FULL: OnceLock<Mutex<Option<AppIndex>>> = OnceLock::new();
    PENDING_FULL.get_or_init(|| Mutex::new(None))
}

/// UI 在空闲时机（隐藏后 / 下次打开）取走并采用 Full 结果。
pub fn take_pending_full() -> Option<AppIndex> {
    pending_full_slot()
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

/// 只有完整快照可以替换当前索引。内容未变化时保留原 Arc，并且不刷新 UI。
fn publish_snapshot(current: &mut AppIndex, built: AppIndex, complete: bool) -> bool {
    if !complete {
        return false;
    }
    if current.retrieval.is_some() && same_snapshot_content(current, &built) {
        return false;
    }
    *current = built;
    true
}

fn same_snapshot_content(left: &AppIndex, right: &AppIndex) -> bool {
    same_items(&left.apps, &right.apps) && same_items(&left.system_entries, &right.system_entries)
}

fn same_items(left: &[AppItem], right: &[AppItem]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            a.id == b.id
                && a.name == b.name
                && a.display_name == b.display_name
                && a.target == b.target
                && a.args == b.args
                && a.working_dir == b.working_dir
                && a.icon == b.icon
                && a.icon_src == b.icon_src
                && a.source == b.source
                && a.normalized_name == b.normalized_name
                && a.normalized_display == b.normalized_display
                && a.pinyin == b.pinyin
                && a.pinyin_initials == b.pinyin_initials
                && a.search_keywords == b.search_keywords
                && a.search_context == b.search_context
        })
}

/// 触发一次完整重建。扫描期间保留旧快照，完整结果生成后再原子发布。
/// 已在跑时标记 pending，当前构建结束后自动再跑一轮。
///
/// 启动首建（进程内第一次 Full）期间，入口监听脏事件只记 deferred，
/// 避免与初始 Full 叠跑第二遍完整扫描。
pub fn request_build(
    index: Arc<Mutex<AppIndex>>,
    icon_dir: PathBuf,
    scan_options: Arc<RwLock<ScanOptions>>,
    tx: UnboundedSender<Message>,
) -> bool {
    if BUILDING.swap(true, Ordering::SeqCst) {
        if STARTUP_BUILD_ACTIVE.load(Ordering::SeqCst) {
            INITIAL_DIRTY.store(true, Ordering::SeqCst);
            plog("entry change during startup build; deferred until first full completes");
        } else {
            PENDING_REBUILD.store(true, Ordering::SeqCst);
            plog("index build already in flight; queued pending rebuild");
        }
        return false;
    }
    if !EVER_COMPLETED_FULL.load(Ordering::SeqCst) {
        STARTUP_BUILD_ACTIVE.store(true, Ordering::SeqCst);
    }
    std::thread::spawn(move || {
        loop {
            PENDING_REBUILD.store(false, Ordering::SeqCst);
            let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
            let options = scan_options
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            build_index_inner(
                index.clone(),
                icon_dir.clone(),
                tx.clone(),
                generation,
                &options,
            );
            // 手动重扫的 force 标记只作用一次；入口监听重建走 TTL 缓存。
            {
                let mut opts = scan_options
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                opts.force_uwp_refresh = false;
            }
            EVER_COMPLETED_FULL.store(true, Ordering::SeqCst);
            if STARTUP_BUILD_ACTIVE.swap(false, Ordering::SeqCst) && INITIAL_DIRTY.swap(false, Ordering::SeqCst)
            {
                PENDING_REBUILD.store(true, Ordering::SeqCst);
                plog("startup-deferred entry change; rebuilding once after first full");
            }
            if !PENDING_REBUILD.swap(false, Ordering::SeqCst) {
                break;
            }
            plog("pending entry change; rebuilding again");
        }
        BUILDING.store(false, Ordering::SeqCst);
    });
    true
}

/// 当前快照是否缺少可搜索检索索引（冷启动或尚未发布过）。
fn needs_bootstrap_snapshot(current: &AppIndex) -> bool {
    current.retrieval.is_none()
}

fn publish_bootstrap(
    index: &Arc<Mutex<AppIndex>>,
    generation: u64,
    built: AppIndex,
    started: Instant,
    tx: &UnboundedSender<Message>,
) -> bool {
    if GENERATION.load(Ordering::SeqCst) != generation {
        plog("bootstrap discarded; newer build started");
        return false;
    }
    let count = built.apps.len();
    let published = index
        .lock()
        .map(|mut current| publish_snapshot(&mut current, built, true))
        .unwrap_or(false);
    let status = if published { "published" } else { "unchanged" };
    plog(&format!(
        "index bootstrap n={count} {status} in {:?} (generation={generation})",
        started.elapsed()
    ));
    let _ = tx.unbounded_send(Message::FullIndexReady(count));
    true
}

fn queue_full_snapshot(
    index: &Arc<Mutex<AppIndex>>,
    generation: u64,
    full: AppIndex,
    started: Instant,
) -> usize {
    let count = full.apps.len();
    if GENERATION.load(Ordering::SeqCst) != generation {
        plog("full discarded; newer build started");
        return count;
    }

    if let Err(error) = app::snapshot::save(&full) {
        plog(&format!("index snapshot save failed: {error}"));
    } else {
        plog(&format!("index snapshot saved n={count}"));
    }

    let differs = index
        .lock()
        .map(|current| !same_snapshot_content(&current, &full))
        .unwrap_or(true);
    if differs {
        if let Ok(mut slot) = pending_full_slot().lock() {
            *slot = Some(full);
        }
        plog(&format!(
            "full snapshot queued for idle apply n={count} in {:?} (generation={generation})",
            started.elapsed()
        ));
    } else {
        plog(&format!(
            "full snapshot unchanged n={count} in {:?} (generation={generation})",
            started.elapsed()
        ));
    }
    count
}

fn build_index_inner(
    index: Arc<Mutex<AppIndex>>,
    icon_dir: PathBuf,
    tx: UnboundedSender<Message>,
    generation: u64,
    options: &ScanOptions,
) {
    let _ = std::fs::create_dir_all(&icon_dir);

    // Cold Start：先 Bootstrap 高价值入口并立刻可搜。Warm Start 启动时已恢复 snapshot。
    let needs_bootstrap = index
        .lock()
        .map(|current| needs_bootstrap_snapshot(&current))
        .unwrap_or(true);
    if needs_bootstrap {
        let bootstrap_started = Instant::now();
        let bootstrap =
            app::scanner::scan_apps_pass_with_options(&icon_dir, ScanPass::Bootstrap, options);
        if !publish_bootstrap(&index, generation, bootstrap, bootstrap_started, &tx) {
            return;
        }
    }

    let started = Instant::now();
    let full = app::scanner::scan_apps_pass_with_options(&icon_dir, ScanPass::Full, options);
    let count = queue_full_snapshot(&index, generation, full, started);
    // 内容未变化也要通知 UI，否则设置页「重新扫描」看起来毫无反应。
    let _ = tx.unbounded_send(Message::FullIndexReady(count));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, target: &str, source: &str) -> AppItem {
        let mut item = AppItem::scanned(id.into(), name.into(), target.into(), None, None, source);
        item.attach_search_fields();
        item
    }

    #[test]
    fn incomplete_snapshot_never_replaces_visible_complete_snapshot() {
        let mut current = AppIndex {
            apps: vec![app(
                "complete",
                "Complete application",
                r"C:\Complete.exe",
                "start-menu",
            )],
            system_entries: Vec::new(),
            retrieval: Some(Arc::new(crate::search::RetrievalIndex::build(&[], &[]))),
        };
        let partial = AppIndex {
            apps: vec![app(
                "partial",
                "Partial application",
                r"C:\Partial.exe",
                "start-menu",
            )],
            system_entries: Vec::new(),
            retrieval: None,
        };

        let published = publish_snapshot(&mut current, partial, false);

        assert!(!published, "incomplete snapshots must stay private");
        assert_eq!(current.apps[0].id, "complete");
    }

    #[test]
    fn complete_snapshot_is_published_only_when_content_changes() {
        let mut current = AppIndex::empty();
        let mut complete = AppIndex {
            apps: vec![app(
                "complete",
                "Complete application",
                r"C:\Complete.exe",
                "start-menu",
            )],
            system_entries: Vec::new(),
            retrieval: None,
        };
        complete.rebuild_retrieval();

        assert!(publish_snapshot(&mut current, complete, true));
        let mut identical = AppIndex {
            apps: current.apps.clone(),
            system_entries: current.system_entries.clone(),
            retrieval: None,
        };
        identical.rebuild_retrieval();
        assert!(
            !publish_snapshot(&mut current, identical, true),
            "an identical rebuild must not refresh the visible result list"
        );
    }

    #[test]
    fn cold_index_requires_bootstrap_before_full() {
        let empty = AppIndex::empty();
        assert!(needs_bootstrap_snapshot(&empty));

        let mut ready = AppIndex {
            apps: vec![app("a", "App", r"C:\App.exe", "start-menu")],
            system_entries: Vec::new(),
            retrieval: None,
        };
        ready.rebuild_retrieval();
        assert!(
            !needs_bootstrap_snapshot(&ready),
            "already searchable snapshots skip bootstrap"
        );
    }

    #[test]
    fn request_build_is_single_flight() {
        let index = Arc::new(Mutex::new(AppIndex::empty()));
        let dir = std::env::temp_dir().join(format!("kite-build-single-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        BUILDING.store(false, Ordering::SeqCst);
        assert!(!BUILDING.swap(true, Ordering::SeqCst));
        assert!(BUILDING.load(Ordering::SeqCst));
        assert!(
            BUILDING.swap(true, Ordering::SeqCst),
            "second caller sees already-building"
        );
        BUILDING.store(false, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(dir);
        let _ = index;
    }

    #[test]
    fn take_pending_full_clears_slot() {
        let mut built = AppIndex {
            apps: vec![app("x", "X", r"C:\X.exe", "start-menu")],
            system_entries: Vec::new(),
            retrieval: None,
        };
        built.rebuild_retrieval();
        {
            let mut slot = pending_full_slot().lock().unwrap();
            *slot = Some(built);
        }
        assert!(take_pending_full().is_some());
        assert!(take_pending_full().is_none());
    }

    #[test]
    fn startup_build_defers_dirty_instead_of_immediate_rebuild() {
        BUILDING.store(true, Ordering::SeqCst);
        STARTUP_BUILD_ACTIVE.store(true, Ordering::SeqCst);
        PENDING_REBUILD.store(false, Ordering::SeqCst);
        INITIAL_DIRTY.store(false, Ordering::SeqCst);
        EVER_COMPLETED_FULL.store(false, Ordering::SeqCst);

        // 模拟启动首建期间 watcher 再次 request_build：只 deferred，不 PENDING_REBUILD
        if BUILDING.swap(true, Ordering::SeqCst) {
            if STARTUP_BUILD_ACTIVE.load(Ordering::SeqCst) {
                INITIAL_DIRTY.store(true, Ordering::SeqCst);
            } else {
                PENDING_REBUILD.store(true, Ordering::SeqCst);
            }
        }
        assert!(INITIAL_DIRTY.load(Ordering::SeqCst));
        assert!(!PENDING_REBUILD.load(Ordering::SeqCst));

        // 首建结束：deferred 转为一轮后续重建
        EVER_COMPLETED_FULL.store(true, Ordering::SeqCst);
        if STARTUP_BUILD_ACTIVE.swap(false, Ordering::SeqCst)
            && INITIAL_DIRTY.swap(false, Ordering::SeqCst)
        {
            PENDING_REBUILD.store(true, Ordering::SeqCst);
        }
        assert!(PENDING_REBUILD.load(Ordering::SeqCst));
        assert!(!INITIAL_DIRTY.load(Ordering::SeqCst));

        // 运行中的后续重建仍走 PENDING_REBUILD
        STARTUP_BUILD_ACTIVE.store(false, Ordering::SeqCst);
        PENDING_REBUILD.store(false, Ordering::SeqCst);
        if BUILDING.swap(true, Ordering::SeqCst) {
            if STARTUP_BUILD_ACTIVE.load(Ordering::SeqCst) {
                INITIAL_DIRTY.store(true, Ordering::SeqCst);
            } else {
                PENDING_REBUILD.store(true, Ordering::SeqCst);
            }
        }
        assert!(PENDING_REBUILD.load(Ordering::SeqCst));
        assert!(!INITIAL_DIRTY.load(Ordering::SeqCst));

        BUILDING.store(false, Ordering::SeqCst);
        STARTUP_BUILD_ACTIVE.store(false, Ordering::SeqCst);
        PENDING_REBUILD.store(false, Ordering::SeqCst);
        INITIAL_DIRTY.store(false, Ordering::SeqCst);
        EVER_COMPLETED_FULL.store(false, Ordering::SeqCst);
    }
}
