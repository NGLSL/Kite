//! 目录遍历：预算/上限约束下的入口收集。

use std::path::Path;
use std::time::{Duration, Instant};

use walkdir::WalkDir;

use crate::model::AppItem;

use super::cache::ScanCache;
use super::url;
use super::util::{self, app_display_name, stable_item_id};
use super::RawItem;

pub(crate) fn budget_exhausted(budget: Option<Duration>, t0: Instant) -> bool {
    budget.is_some_and(|b| t0.elapsed() >= b)
}

pub(crate) fn collect_from_dir(
    root: &Path,
    source: &str,
    max_depth: usize,
    budget: Option<Duration>,
    t0: Instant,
    max_per_dir: usize,
    max_total: usize,
    out: &mut Vec<RawItem>,
    cache: &mut ScanCache,
) {
    if !root.exists() {
        return;
    }
    let mut n = 0usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if n >= max_per_dir || out.len() >= max_total || budget_exhausted(budget, t0) {
            if n >= max_per_dir {
                crate::log::info(&format!(
                    "scan per-dir cap {max_per_dir} in {} ({source})",
                    root.display()
                ));
            }
            if out.len() >= max_total {
                crate::log::info(&format!(
                    "scan total cap {max_total} while walking {} ({source})",
                    root.display()
                ));
            }
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
            // 缓存命中跳过 COM 解析（fast 扫描的大头）
            let resolved = if let Some(hit) = cache.lookup(path) {
                Some(hit)
            } else {
                let parsed = super::lnk::resolve_lnk(path);
                if let Some(r) = &parsed {
                    cache.record(path, r);
                }
                parsed
            };
            if let Some((target, args, working_dir, icon_src)) = resolved {
                let name = app_display_name(&file_name);
                let id = stable_item_id(&target, args.as_deref());
                let mut item = AppItem::scanned(id, name, target, args, working_dir, source);
                item.is_lnk = true;
                out.push((item, icon_src));
                n += 1;
            }
        } else if ext == "url" && url::is_app_protocol_shortcut(path) {
            // ShellExecute opens the verified shortcut file, so the parsed URL
            // never becomes an executable target or a shell command.
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = stable_item_id(&target, None);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, None));
            n += 1;
        } else if ext == "appref-ms" {
            // ClickOnce application references are themselves verified files;
            // Windows Shell resolves deployment metadata when the user opens it.
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = stable_item_id(&target, None);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, None));
            n += 1;
        } else if ext == "exe" {
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = stable_item_id(&target, None);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let icon_src = Some(target.clone());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, icon_src));
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::scanner::pass::{
        FULL_MAX_PER_DIR, FULL_MAX_TOTAL, FULL_START_MENU_MAX_DEPTH, START_MENU_MAX_DEPTH,
    };
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kite-walk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nested_program_entries_are_indexed() {
        let root = temp_dir("nested-programs");
        let first = root.join("Programs").join("Category").join("Launcher.exe");
        let second = root
            .join("Programs")
            .join("Category")
            .join("Tools")
            .join("Developer Launcher.exe");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::write(&first, b"fixture").unwrap();
        std::fs::write(&second, b"fixture").unwrap();

        let mut raw = Vec::new();
        let mut cache = ScanCache::load(&root);
        collect_from_dir(
            &root,
            "start-menu",
            START_MENU_MAX_DEPTH,
            None,
            Instant::now(),
            super::super::pass::MAX_PER_DIR,
            super::super::pass::MAX_TOTAL,
            &mut raw,
            &mut cache,
        );

        let targets: Vec<_> = raw.iter().map(|(item, _)| item.target.as_str()).collect();
        assert!(targets.contains(&first.to_string_lossy().as_ref()));
        assert!(targets.contains(&second.to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn protocol_shortcut_in_start_menu_is_searchable() {
        let root = temp_dir("protocol-shortcut");
        let shortcut = root.join("Example Game.url");
        let website = root.join("Example Website.url");
        std::fs::write(
            &shortcut,
            b"[{000214A0-0000-0000-C000-000000000046}]\r\nProp3=19,0\r\n[InternetShortcut]\r\nURL=steam://rungameid/123\r\n",
        )
        .unwrap();
        std::fs::write(
            &website,
            b"[InternetShortcut]\r\nURL=https://example.com\r\n",
        )
        .unwrap();

        let mut raw = Vec::new();
        let mut cache = ScanCache::load(&root);
        collect_from_dir(
            &root,
            "start-menu",
            START_MENU_MAX_DEPTH,
            None,
            Instant::now(),
            super::super::pass::MAX_PER_DIR,
            super::super::pass::MAX_TOTAL,
            &mut raw,
            &mut cache,
        );
        assert!(
            raw.iter()
                .all(|(item, _)| item.target != website.to_string_lossy()),
            "web bookmarks should not be indexed as installed software"
        );
        let (mut item, _) = raw
            .into_iter()
            .find(|(item, _)| item.target == shortcut.to_string_lossy())
            .expect("installed game protocol shortcut must be indexed");
        item.attach_search_fields();
        let retrieval = crate::search::RetrievalIndex::build(&[item], &[]);
        assert!(
            retrieval
                .search("Example Game", &[], 10)
                .iter()
                .any(|hit| hit.item.name == "Example Game"),
            "shortcut must be searchable by its display name"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clickonce_shortcut_in_start_menu_is_searchable() {
        let root = temp_dir("clickonce-shortcut");
        let shortcut = root.join("Enterprise Portal.appref-ms");
        std::fs::write(&shortcut, b"fixture").unwrap();
        let mut raw = Vec::new();
        let mut cache = ScanCache::load(&root);
        collect_from_dir(
            &root,
            "start-menu",
            START_MENU_MAX_DEPTH,
            None,
            Instant::now(),
            super::super::pass::MAX_PER_DIR,
            super::super::pass::MAX_TOTAL,
            &mut raw,
            &mut cache,
        );
        let mut item = raw
            .into_iter()
            .map(|(item, _)| item)
            .find(|item| item.target == shortcut.to_string_lossy())
            .expect("ClickOnce application reference must be indexed");
        item.attach_search_fields();
        let retrieval = crate::search::RetrievalIndex::build(&[item], &[]);
        assert!(retrieval
            .search("Enterprise Portal", &[], 10)
            .iter()
            .any(|hit| { hit.item.target == shortcut.to_string_lossy() }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn full_pass_recurses_deeper_than_fast_depth() {
        let root = temp_dir("full-depth");
        let shallow = root.join("a").join("b").join("c").join("Shallow.exe");
        let deep = root
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("e")
            .join("Deep.exe");
        std::fs::create_dir_all(shallow.parent().unwrap()).unwrap();
        std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
        std::fs::write(&shallow, b"fixture").unwrap();
        std::fs::write(&deep, b"fixture").unwrap();

        let collect_at = |depth: usize| {
            let mut raw = Vec::new();
            let mut cache = ScanCache::load(&root);
            collect_from_dir(
                &root,
                "start-menu",
                depth,
                None,
                Instant::now(),
                FULL_MAX_PER_DIR,
                FULL_MAX_TOTAL,
                &mut raw,
                &mut cache,
            );
            raw.iter()
                .map(|(item, _)| item.target.clone())
                .collect::<Vec<_>>()
        };

        let fast_hits = collect_at(START_MENU_MAX_DEPTH);
        let full_hits = collect_at(FULL_START_MENU_MAX_DEPTH);
        assert!(
            fast_hits.contains(&shallow.to_string_lossy().to_string()),
            "fast depth should still see common nested entry"
        );
        assert!(
            !fast_hits.contains(&deep.to_string_lossy().to_string()),
            "fast depth stops before sixth-level entry"
        );
        assert!(
            full_hits.contains(&deep.to_string_lossy().to_string()),
            "full pass must reach deeper than fast budget/depth"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
