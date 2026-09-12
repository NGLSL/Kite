//! 扫描开始菜单 / 桌面，解析快捷方式，去重。
//! 注意：不要 follow_links（开始菜单 junction 可能卡死）；快速扫描有时间预算。

mod lnk;
mod registry;
pub(crate) mod util;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use walkdir::WalkDir;

use crate::model::{AppIndex, AppItem};
use crate::system::icons;
use util::{app_display_name, hash_id, normalize_path_key};

type RawItem = (AppItem, Option<String>);

/// 快速扫描时间预算；超时后用已收集结果。
const FAST_BUDGET: Duration = Duration::from_millis(1500);
const MAX_PER_DIR: usize = 400;
const MAX_TOTAL: usize = 1500;

/// 完整扫描。`fast` 为 true 时只建列表、不提图标（首屏用）。
pub fn scan_apps(icon_dir: &Path, fast: bool) -> AppIndex {
    let t0 = Instant::now();
    let budget = if fast { Some(FAST_BUDGET) } else { None };
    let mut raw: Vec<RawItem> = Vec::new();

    let user_start = dirs::data_dir()
        .map(|d| d.join("Microsoft/Windows/Start Menu"))
        .unwrap_or_default();
    let common_start = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu");
    let user_desktop = dirs::desktop_dir().unwrap_or_default();
    let public_desktop = PathBuf::from(r"C:\Users\Public\Desktop");

    crate::log::info(&format!("scan fast={fast} start"));

    let steps: [(&str, PathBuf, &str); 4] = [
        ("user-start", user_start, "start-menu"),
        ("common-start", common_start, "start-menu"),
        ("user-desktop", user_desktop, "desktop"),
        ("public-desktop", public_desktop, "desktop"),
    ];

    for (label, root, source) in steps {
        if budget_exhausted(budget, t0) {
            crate::log::info(&format!("scan budget hit before {label}"));
            break;
        }
        let before = raw.len();
        collect_from_dir(&root, source, budget, t0, &mut raw);
        crate::log::info(&format!(
            "{label}: +{} -> total {} in {:?}",
            raw.len() - before,
            raw.len(),
            t0.elapsed()
        ));
    }

    if !budget_exhausted(budget, t0) && raw.len() < MAX_TOTAL {
        let t = Instant::now();
        registry::collect_app_paths("app-paths", &mut raw);
        crate::log::info(&format!(
            "app-paths: +... total {} in {:?}",
            raw.len(),
            t.elapsed()
        ));
    }

    // UWP 较慢且噪声多，仅在非快速扫描时做，且限制数量
    if !fast {
        let t = Instant::now();
        crate::app::uwp::collect_uwp("uwp", &mut raw);
        crate::log::info(&format!(
            "uwp: total {} in {:?}",
            raw.len(),
            t.elapsed()
        ));
    }

    let items = dedupe(raw);
    crate::log::info(&format!("dedupe -> {} in {:?}", items.len(), t0.elapsed()));

    let mut apps = Vec::with_capacity(items.len());
    let t = Instant::now();
    let mut py = Duration::ZERO;
    for (mut item, icon_src) in items {
        let tp = Instant::now();
        item.attach_search_fields();
        py += tp.elapsed();
        // 保留提取源，快速扫描阶段先不提图标，稍后统一补
        item.icon_src = icon_src.or_else(|| Some(item.target.clone()));
        if !fast {
            item.icon = icons::cache_icon(icon_dir, &item.id, item.icon_src.as_deref());
        }
        apps.push(item);
    }
    crate::log::info(&format!(
        "fields/icons fast={fast}: {} apps in {:?} pinyin={py:?}",
        apps.len(),
        t.elapsed()
    ));

    AppIndex { apps }
}

fn budget_exhausted(budget: Option<Duration>, t0: Instant) -> bool {
    budget.is_some_and(|b| t0.elapsed() >= b)
}

/// 收集缺少图标的条目 (id, 提取源)。调用方短暂持锁后即可释放。
pub fn missing_icon_targets(index: &AppIndex) -> Vec<(String, Option<String>)> {
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
}

/// 并行提取图标，返回 id → PNG 路径；提取过程不持任何锁，调用方拿结果后短暂持锁合并。
pub fn extract_icons_parallel(
    pending: &[(String, Option<String>)],
    icon_dir: &Path,
) -> HashMap<String, Option<String>> {
    use std::sync::Arc;
    let results = Arc::new(Mutex::new(HashMap::<String, Option<String>>::new()));
    if pending.is_empty() {
        return HashMap::new();
    }
    let chunk = pending.len().div_ceil(8).max(1);

    std::thread::scope(|s| {
        for part in pending.chunks(chunk) {
            let part: Vec<(String, Option<String>)> = part.to_vec();
            let icon_dir = icon_dir.to_path_buf();
            let results = Arc::clone(&results);
            s.spawn(move || {
                for (id, src) in &part {
                    let path = icons::cache_icon(&icon_dir, id, src.as_deref());
                    if let Ok(mut g) = results.lock() {
                        g.insert(id.clone(), path);
                    }
                }
            });
        }
    });

    results.lock().map(|g| g.clone()).unwrap_or_default()
}

fn collect_from_dir(
    root: &Path,
    source: &str,
    budget: Option<Duration>,
    t0: Instant,
    out: &mut Vec<RawItem>,
) {
    if !root.exists() {
        return;
    }
    let mut n = 0usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if n >= MAX_PER_DIR || out.len() >= MAX_TOTAL || budget_exhausted(budget, t0) {
            break;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if util::is_skippable_shortcut(&file_name) {
            continue;
        }

        if ext == "lnk" {
            if let Some((target, args, working_dir, icon_src)) = lnk::resolve_lnk(path) {
                let name = app_display_name(&file_name);
                let id = hash_id(&[
                    &normalize_path_key(&target),
                    args.as_deref().unwrap_or(""),
                    source,
                ]);
                let item = AppItem::scanned(id, name, target, args, working_dir, source);
                out.push((item, icon_src));
                n += 1;
            }
        } else if ext == "exe" {
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = hash_id(&[&normalize_path_key(&target), source]);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let icon_src = Some(target.clone());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, icon_src));
            n += 1;
        }
    }
}

fn dedupe(raw: Vec<RawItem>) -> Vec<RawItem> {
    let rank = |source: &str| match source {
        "start-menu" => 0,
        "desktop" => 1,
        "app-paths" => 2,
        "uwp" => 3,
        _ => 4,
    };

    let mut best: HashMap<String, RawItem> = HashMap::new();
    for (item, icon) in raw {
        let key = normalize_path_key(&item.target);
        match best.get(&key) {
            Some((existing, _)) if rank(&existing.source) <= rank(&item.source) => {}
            _ => {
                best.insert(key, (item, icon));
            }
        }
    }

    let mut list: Vec<_> = best.into_values().collect();
    list.sort_by(|a, b| {
        a.0.name
            .to_lowercase()
            .cmp(&b.0.name.to_lowercase())
            .then_with(|| a.0.source.cmp(&b.0.source))
    });
    list
}
