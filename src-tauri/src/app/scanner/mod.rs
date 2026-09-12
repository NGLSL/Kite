//! 扫描开始菜单 / 桌面，解析快捷方式，去重并挂载图标。
//! 性能：先出完整列表（不含图标），再并行补图标，避免首屏卡在「正在扫描」。

mod lnk;
mod registry;
pub(crate) mod util;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use walkdir::WalkDir;

use crate::model::{AppIndex, AppItem};
use crate::system::icons;
use util::{app_display_name, hash_id, normalize_path_key};

type RawItem = (AppItem, Option<String>);

/// 完整扫描。`fast` 为 true 时只建列表、不提图标（首屏用）。
pub fn scan_apps(icon_dir: &Path, fast: bool) -> AppIndex {
    let mut raw: Vec<RawItem> = Vec::new();

    let user_start = dirs::data_dir()
        .map(|d| d.join("Microsoft/Windows/Start Menu"))
        .unwrap_or_default();
    let common_start = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu");
    let user_desktop = dirs::desktop_dir().unwrap_or_default();
    let public_desktop = PathBuf::from(r"C:\Users\Public\Desktop");

    collect_from_dir(&user_start, "start-menu", &mut raw);
    collect_from_dir(&common_start, "start-menu", &mut raw);
    collect_from_dir(&user_desktop, "desktop", &mut raw);
    collect_from_dir(&public_desktop, "desktop", &mut raw);
    registry::collect_app_paths("app-paths", &mut raw);
    crate::app::uwp::collect_uwp("uwp", &mut raw);

    let items = dedupe(raw);
    let mut apps = Vec::with_capacity(items.len());

    for (mut item, icon_src) in items {
        item.attach_search_fields();
        if !fast {
            item.icon = icons::cache_icon(icon_dir, &item.id, icon_src.as_deref());
        } else {
            // 快速路径：记下来源，稍后 fill_missing_icons 用 target 补图标
            let _ = icon_src;
        }
        apps.push(item);
    }

    AppIndex { apps }
}

/// 只给缺少 icon 的条目补图标（可多次调用）。
pub fn fill_missing_icons(index: &mut AppIndex, icon_dir: &Path) {
    let pending: Vec<(String, Option<String>)> = index
        .apps
        .iter()
        .filter(|a| a.icon.is_none())
        .map(|a| (a.id.clone(), Some(a.target.clone())))
        .collect();
    if pending.is_empty() {
        return;
    }
    fill_icons_parallel(&mut index.apps, icon_dir, &pending);
}

fn fill_icons_parallel(apps: &mut [AppItem], icon_dir: &Path, pending: &[(String, Option<String>)]) {
    use std::sync::Arc;
    let results = Arc::new(Mutex::new(HashMap::<String, Option<String>>::new()));
    let chunk = pending.len().div_ceil(8).max(1);
    let icon_dir = icon_dir.to_path_buf();

    std::thread::scope(|s| {
        for part in pending.chunks(chunk) {
            let part: Vec<(String, Option<String>)> = part.to_vec();
            let icon_dir = icon_dir.clone();
            let results = Arc::clone(&results);
            s.spawn(move || {
                let mut local = Vec::with_capacity(part.len());
                for (id, src) in &part {
                    let path = icons::cache_icon(&icon_dir, id, src.as_deref());
                    local.push((id.clone(), path));
                }
                if let Ok(mut g) = results.lock() {
                    for (id, path) in local {
                        g.insert(id, path);
                    }
                }
            });
        }
    });

    let snapshot: HashMap<String, Option<String>> = results
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    for app in apps.iter_mut() {
        if app.icon.is_none() {
            if let Some(p) = snapshot.get(&app.id) {
                app.icon = p.clone();
            }
        }
    }
}

fn collect_from_dir(root: &Path, source: &str, out: &mut Vec<RawItem>) {
    if !root.exists() {
        return;
    }
    for entry in WalkDir::new(root)
        .follow_links(true)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
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
            }
        } else if ext == "exe" {
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = hash_id(&[&normalize_path_key(&target), source]);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let icon_src = Some(target.clone());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, icon_src));
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
