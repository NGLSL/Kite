//! 扫描开始菜单 / 桌面，解析快捷方式，去重并挂载图标。

mod lnk;
mod registry;
mod util;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::model::{AppIndex, AppItem};
use crate::system::icons;
use util::{app_display_name, hash_id, normalize_path_key};

type RawItem = (AppItem, Option<String>);

/// 启动与手动重扫使用的完整扫描。
pub fn scan_apps(icon_dir: &Path) -> AppIndex {
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

    let items = dedupe(raw);
    let mut apps = Vec::with_capacity(items.len());
    for (mut item, icon_src) in items {
        item.icon = icons::cache_icon(icon_dir, &item.id, icon_src.as_deref());
        // 索引阶段预计算规范化名与拼音，搜索热路径只做字符串比较
        item.attach_search_fields();
        apps.push(item);
    }

    AppIndex { apps }
}

fn collect_from_dir(root: &Path, source: &str, out: &mut Vec<RawItem>) {
    if !root.exists() {
        return;
    }
    for entry in WalkDir::new(root)
        .follow_links(true)
        .max_depth(4)
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
        _ => 3,
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
