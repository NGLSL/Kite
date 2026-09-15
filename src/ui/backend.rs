//! 后台索引重建。
//!
//! 扫描在线程内构建完整搜索快照，完成后一次性替换 UI 当前快照。
//! 同一时刻最多一次构建；构建期间继续使用上一份完整快照。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
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
pub fn request_build(
    index: Arc<Mutex<AppIndex>>,
    icon_dir: PathBuf,
    scan_options: Arc<RwLock<ScanOptions>>,
    tx: UnboundedSender<Message>,
) -> bool {
    if BUILDING.swap(true, Ordering::SeqCst) {
        PENDING_REBUILD.store(true, Ordering::SeqCst);
        plog("index build already in flight; queued pending rebuild");
        return false;
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
            if !PENDING_REBUILD.swap(false, Ordering::SeqCst) {
                break;
            }
            plog("pending entry change; rebuilding again");
        }
        BUILDING.store(false, Ordering::SeqCst);
    });
    true
}

fn build_index_inner(
    index: Arc<Mutex<AppIndex>>,
    icon_dir: PathBuf,
    tx: UnboundedSender<Message>,
    generation: u64,
    options: &ScanOptions,
) {
    let _ = std::fs::create_dir_all(&icon_dir);
    let started = Instant::now();
    let full = app::scanner::scan_apps_pass_with_options(&icon_dir, ScanPass::Full, options);
    let count = full.apps.len();
    if GENERATION.load(Ordering::SeqCst) != generation {
        plog("complete snapshot discarded; newer build started");
        return;
    }

    let published = index
        .lock()
        .map(|mut current| publish_snapshot(&mut current, full, true))
        .unwrap_or(false);
    let status = if published { "published" } else { "unchanged" };
    plog(&format!(
        "index complete n={count} {status} in {:?} (generation={generation})",
        started.elapsed()
    ));
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
}
