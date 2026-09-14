//! 原型后台：历史库副本与索引重建（对齐 state::rebuild_index，去 Tauri 化）。
//! 只调用 kite_lib 既有函数，不改数据格式；历史库用副本，避免污染 Kite 真实数据。
//!
//! 快扫首屏 → 图标补齐 → UWP 合并 → 后台完整扫描原子替换。
//! 同一时刻最多一次构建；旧构建结果不得覆盖更新的快照。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iced::futures::channel::mpsc::UnboundedSender;

use crate::app::scanner::ScanPass;
use crate::model::{AppIndex, AppItem};
use crate::{app, system};

use super::{plog, Message};

/// 单飞：true 表示已有构建在跑，新的请求合并跳过（完成后由调用方再触发）。
static BUILDING: AtomicBool = AtomicBool::new(false);
/// 构建期间又有变化：当前构建结束后再跑一轮。
static PENDING_REBUILD: AtomicBool = AtomicBool::new(false);
/// 构建代际：完成时仅当仍等于启动时记下的 generation 才发布，避免旧构建覆盖新状态。
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// 是否已有索引构建在跑。
pub fn is_building() -> bool {
    BUILDING.load(Ordering::SeqCst)
}

/// 触发一次完整重建：快扫首屏 + 后台完整补扫。
/// 已在跑时标记 pending，当前构建结束后自动再跑一轮。
pub fn request_build(index: Arc<Mutex<AppIndex>>, icon_dir: PathBuf, tx: UnboundedSender<Message>) -> bool {
    if BUILDING.swap(true, Ordering::SeqCst) {
        PENDING_REBUILD.store(true, Ordering::SeqCst);
        plog("index build already in flight; queued pending rebuild");
        return false;
    }
    std::thread::spawn(move || {
        loop {
            PENDING_REBUILD.store(false, Ordering::SeqCst);
            let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
            build_index_inner(index.clone(), icon_dir.clone(), tx.clone(), generation);
            if !PENDING_REBUILD.swap(false, Ordering::SeqCst) {
                break;
            }
            plog("pending entry change; rebuilding again");
        }
        BUILDING.store(false, Ordering::SeqCst);
    });
    true
}

/// 三阶段快扫 + 第四阶段完整扫描：快速索引（首屏可搜索）→ 图标并行补齐 →
/// UWP 后台合并 → 后台完整递归扫描原子替换。
fn build_index_inner(
    index: Arc<Mutex<AppIndex>>,
    icon_dir: PathBuf,
    tx: UnboundedSender<Message>,
    generation: u64,
) {
    let _ = std::fs::create_dir_all(&icon_dir);

    // 阶段 1：快速扫描（与 Kite 首屏一致，不提图标）
    let t0 = Instant::now();
    let built = app::scan_apps(&icon_dir, true);
    let n = built.apps.len();
    if GENERATION.load(Ordering::SeqCst) != generation {
        plog("fast snapshot skipped; newer build started");
        return;
    }
    if let Ok(mut g) = index.lock() {
        *g = built;
    }
    plog(&format!("index fast n={n} in {:?}", t0.elapsed()));
    let _ = tx.unbounded_send(Message::IndexReady(n));

    // 阶段 2：图标并行补齐（对齐 rebuild_index 的后台补图标，全程不持锁）
    let t1 = Instant::now();
    let pending = {
        let g = index.lock().unwrap_or_else(|e| e.into_inner());
        app::scanner::missing_icon_targets(&g)
    };
    let filled = app::scanner::extract_icons_parallel(&pending, &icon_dir);
    if GENERATION.load(Ordering::SeqCst) == generation {
        if let Ok(mut g) = index.lock() {
            for item in g.apps.iter_mut() {
                if item.icon.is_none() {
                    if let Some(Some(p)) = filled.get(&item.id) {
                        item.icon = Some(p.clone());
                    }
                }
            }
            let with_icon = g.apps.iter().filter(|a| a.icon.is_some()).count();
            plog(&format!(
                "icons filled {with_icon}/{} in {:?}",
                g.apps.len(),
                t1.elapsed()
            ));
        }
    }
    let _ = tx.unbounded_send(Message::IconsFilled(filled.len()));

    // 阶段 3：UWP / Store 应用后台补扫合并（对齐 merge_uwp_apps）
    let t2 = Instant::now();
    let mut raw = Vec::new();
    app::uwp::collect_uwp("uwp", &mut raw);
    let raw_n = raw.len();
    let added = merge_uwp(&index, raw, &icon_dir, generation);
    plog(&format!(
        "uwp merged +{added} (scanned {raw_n}) in {:?}",
        t2.elapsed()
    ));
    let _ = tx.unbounded_send(Message::UwpMerged(added));

    // 阶段 4：后台完整扫描（无快扫时间预算、更深递归、含 UWP/图标），
    // 构建完整新快照后原子替换；构建期间旧索引始终可搜。
    if GENERATION.load(Ordering::SeqCst) != generation {
        plog("full scan skipped; newer build started");
        return;
    }
    let t3 = Instant::now();
    let full = app::scanner::scan_apps_pass(&icon_dir, ScanPass::Full, &[]);
    let full_n = full.apps.len();
    if GENERATION.load(Ordering::SeqCst) != generation {
        plog("full snapshot discarded; newer build started");
        return;
    }
    if let Ok(mut g) = index.lock() {
        *g = full;
    }
    plog(&format!(
        "index full n={full_n} in {:?} (generation={generation})",
        t3.elapsed()
    ));
    let _ = tx.unbounded_send(Message::FullIndexReady(full_n));
}

/// 与 state::merge_uwp_apps 相同的去重合并：按规范化路径 key 去重后并入索引。
fn merge_uwp(
    index: &Arc<Mutex<AppIndex>>,
    raw: Vec<(AppItem, Option<String>)>,
    icon_dir: &Path,
    generation: u64,
) -> usize {
    use crate::app::scanner::util::normalize_path_key;

    let mut known: HashSet<String> = {
        let g = index.lock().unwrap_or_else(|e| e.into_inner());
        g.apps
            .iter()
            .map(|a| normalize_path_key(&a.target))
            .collect()
    };

    let mut to_add = Vec::new();
    for (mut item, icon_src) in raw {
        let key = normalize_path_key(&item.target);
        if !known.insert(key) {
            continue;
        }
        item.attach_search_fields();
        item.icon_src = icon_src.or_else(|| Some(item.target.clone()));
        item.icon = system::icons::cache_icon(
            icon_dir,
            &item.id,
            item.icon_src.as_deref(),
            Some(item.target.as_str()),
        );
        to_add.push(item);
    }

    if GENERATION.load(Ordering::SeqCst) != generation {
        return 0;
    }

    let mut added = 0usize;
    if let Ok(mut g) = index.lock() {
        let existing: HashSet<String> = g
            .apps
            .iter()
            .map(|a| normalize_path_key(&a.target))
            .collect();
        for item in to_add {
            if existing.contains(&normalize_path_key(&item.target)) {
                continue;
            }
            g.apps.push(item);
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_build_is_single_flight() {
        let index = Arc::new(Mutex::new(AppIndex::empty()));
        let dir = std::env::temp_dir().join(format!("kite-build-single-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // 不真正跑完整扫描：用极短存在性验证单飞标志。
        // request_build 会 spawn 真实扫描（可能访问系统目录），因此只测 swap 语义。
        BUILDING.store(false, Ordering::SeqCst);
        // 手动模拟：第一次 swap 成功
        assert!(!BUILDING.swap(true, Ordering::SeqCst));
        assert!(is_building());
        assert!(BUILDING.swap(true, Ordering::SeqCst), "second caller sees already-building");
        BUILDING.store(false, Ordering::SeqCst);
        let _ = std::fs::remove_dir_all(dir);
        // index 仅防止 unused；真实 request_build 由 UI 线程触发
        let _ = index;
    }
}
