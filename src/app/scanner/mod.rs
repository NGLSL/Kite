//! 扫描编排：按档位收集入口、去重、附着搜索字段并发布 AppIndex。
//!
//! 模块职责：
//! - `pass`：扫描档位与预算
//! - `walk`：目录遍历与入口解析
//! - `scoop`：Scoop shims 根目录
//! - `merge`：身份去重与命令别名归并
//! - `icon_batch`：图标批处理
//! - `commands` / `registry` / `uninstall` / `protocols` / `uwp` 等：各来源

mod cache;
mod commands;
mod family;
mod icon_batch;
mod lnk;
mod merge;
mod metadata;
mod pass;
mod protocols;
mod registry;
mod scoop;
mod uninstall;
mod url;
pub mod util;
mod walk;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::model::{AppIndex, AppItem};
use cache::UwpCache;
use merge::{
    absorb_discovery_rows, dedupe, exclude_system_name_duplicates, merge_command_name_duplicates,
    merge_same_name_formal_duplicates,
};
use pass::FAST_BUDGET;
use scoop::collect_scoop_shims;
use walk::{budget_exhausted, collect_from_dir};

pub use family::{
    discovery_absorbs_into, exe_stem_matches_display_family, exe_stem_links_to_compact_name,
    is_shell_package, should_merge_family, strip_shell_target,
};
pub use icon_batch::{extract_icons_parallel, missing_icon_targets};
pub use pass::{ScanOptions, ScanPass};

type RawItem = (AppItem, Option<String>);

/// 完整扫描。`fast` 为 true 时只建列表、不提图标（兼容路径）。
pub fn scan_apps(icon_dir: &Path, fast: bool) -> AppIndex {
    let pass = if fast { ScanPass::Fast } else { ScanPass::Full };
    scan_apps_pass(icon_dir, pass, &[])
}

/// 按档位扫描；完整档可作为后台第二遍原子替换首屏快照。
pub fn scan_apps_pass(
    icon_dir: &Path,
    pass: ScanPass,
    extra_scoop_shim_dirs: &[PathBuf],
) -> AppIndex {
    scan_apps_pass_with_options(
        icon_dir,
        pass,
        &ScanOptions {
            extra_scoop_shim_dirs: extra_scoop_shim_dirs.to_vec(),
            portable_dirs: Vec::new(),
            force_uwp_refresh: false,
        },
    )
}

pub fn scan_apps_pass_with_options(
    icon_dir: &Path,
    pass: ScanPass,
    options: &ScanOptions,
) -> AppIndex {
    scan_apps_with_options(icon_dir, pass, options)
}

/// 监听命令目录的直接子项，以及 Codex Desktop 的版本目录变化。
pub fn command_watch_roots() -> Vec<PathBuf> {
    let mut roots = command_scan_roots();
    if let Some(codex_bin) = commands::codex_bin_root() {
        roots.push(codex_bin);
    }
    roots
}

pub fn command_scan_roots() -> Vec<PathBuf> {
    commands::configured_command_roots()
}

fn scan_apps_with_options(icon_dir: &Path, pass: ScanPass, options: &ScanOptions) -> AppIndex {
    let budget = pass.timed_budget().then_some(FAST_BUDGET);
    scan_apps_with_budget(icon_dir, pass, options, budget)
}

fn scan_apps_with_budget(
    icon_dir: &Path,
    pass: ScanPass,
    options: &ScanOptions,
    budget: Option<Duration>,
) -> AppIndex {
    let t0 = Instant::now();
    let start_depth = pass.start_menu_depth();
    let other_depth = pass.other_source_depth();
    let max_per_dir = pass.max_per_dir();
    let max_total = pass.max_total();
    let mut raw: Vec<RawItem> = Vec::new();
    let mut cache = cache::ScanCache::load(icon_dir);

    let user_start = util::user_start_menu_dir()
        .or_else(|| dirs::data_dir().map(|d| d.join("Microsoft/Windows/Start Menu")))
        .unwrap_or_default();
    let common_start = util::common_start_menu_dir()
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu"));
    let user_desktop = dirs::desktop_dir().unwrap_or_default();
    let public_desktop = PathBuf::from(r"C:\Users\Public\Desktop");
    let user_home = dirs::home_dir();
    let program_data = std::env::var_os("ProgramData").map(PathBuf::from);
    let scoop_root = std::env::var_os("SCOOP").map(PathBuf::from);
    let scoop_global_root = std::env::var_os("SCOOP_GLOBAL").map(PathBuf::from);

    crate::log::info(&format!("scan pass={pass:?} start"));

    // Full：UWP/Store 枚举（COM，较慢）与目录扫描并行。
    // UWP 仅 Full 收集；30min TTL 内自动复用，手动「重新扫描」强制刷新。
    // 不要对 Full 无条件 force——那会让 UWP 缓存永远 miss。
    let uwp_force = options.force_uwp_refresh;
    let uwp_icon_dir = icon_dir.to_path_buf();
    let uwp_worker = if pass.include_uwp() {
        Some(std::thread::spawn(move || {
            let t = Instant::now();
            let cache = UwpCache::load(&uwp_icon_dir);
            let now = crate::storage::now_ts().max(0) as u64;
            if let Some(cached) = cache.fresh_items(uwp_force, now) {
                crate::log::info(&format!(
                    "uwp cache hit: {} items (force={uwp_force}) in {:?}",
                    cached.len(),
                    t.elapsed()
                ));
                return cached
                    .into_iter()
                    .map(|c| {
                        let source = if c.source.is_empty() {
                            "uwp"
                        } else {
                            c.source.as_str()
                        };
                        let mut item = AppItem::scanned(
                            c.id,
                            c.name,
                            c.target,
                            None,
                            None,
                            source,
                        );
                        item.attach_search_fields();
                        (item, c.icon_src)
                    })
                    .collect::<Vec<_>>();
            }
            let mut items = Vec::new();
            crate::app::uwp::collect_uwp("uwp", &mut items);
            let snapshot: Vec<cache::CachedUwpItem> = items
                .iter()
                .map(|(item, icon_src)| cache::CachedUwpItem {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    target: item.target.clone(),
                    source: item.source.clone(),
                    icon_src: icon_src.clone(),
                })
                .collect();
            cache.save(snapshot, now);
            crate::log::info(&format!(
                "uwp fresh: {} items in {:?}",
                items.len(),
                t.elapsed()
            ));
            items
        }))
    } else {
        None
    };

    if pass.include_supplemental_sources() {
        let before = raw.len();
        collect_scoop_shims(
            user_home.as_deref(),
            program_data.as_deref(),
            scoop_root.as_deref(),
            scoop_global_root.as_deref(),
            &options.extra_scoop_shim_dirs,
            budget,
            t0,
            other_depth,
            max_per_dir,
            max_total,
            &mut raw,
            &mut cache,
        );
        crate::log::info(&format!(
            "scoop-shims: +{} -> total {} in {:?}",
            raw.len() - before,
            raw.len(),
            t0.elapsed()
        ));
    }

    // Start Menu/Desktop 优先，Portable 其后：避免大 Portable 目录吃光 Bootstrap 预算。
    let steps: [(&str, PathBuf, &str, usize); 4] = [
        ("user-start", user_start, "start-menu", start_depth),
        ("common-start", common_start, "start-menu", start_depth),
        ("user-desktop", user_desktop, "desktop", other_depth),
        ("public-desktop", public_desktop, "desktop", other_depth),
    ];

    for (label, root, source, max_depth) in steps {
        if budget_exhausted(budget, t0) {
            crate::log::info(&format!("scan budget hit before {label}"));
            break;
        }
        if raw.len() >= max_total {
            crate::log::info(&format!(
                "scan safety cap max_total={max_total} before {label}; uncovered dir recorded"
            ));
            break;
        }
        let before = raw.len();
        collect_from_dir(
            &root,
            source,
            max_depth,
            budget,
            t0,
            max_per_dir,
            max_total,
            &mut raw,
            &mut cache,
        );
        crate::log::info(&format!(
            "{label}: +{} -> total {} in {:?}",
            raw.len() - before,
            raw.len(),
            t0.elapsed()
        ));
    }

    // 用户 Portable：Tier A 正式入口，Bootstrap/Full 都收集；排在 Start Menu/Desktop 之后。
    for root in &options.portable_dirs {
        if budget_exhausted(budget, t0) || raw.len() >= max_total {
            break;
        }
        let before = raw.len();
        collect_from_dir(
            root,
            "portable",
            other_depth,
            budget,
            t0,
            max_per_dir,
            max_total,
            &mut raw,
            &mut cache,
        );
        crate::log::info(&format!(
            "portable {}: +{} -> total {} in {:?}",
            root.display(),
            raw.len() - before,
            raw.len(),
            t0.elapsed()
        ));
    }

    if pass.include_supplemental_sources() {
        let before = raw.len();
        commands::collect_command_entries(commands::COMMAND_SOURCE, &mut raw);
        crate::log::info(&format!(
            "command aliases: +{} -> total {} in {:?}",
            raw.len() - before,
            raw.len(),
            t0.elapsed()
        ));

        let before = raw.len();
        raw.extend(uninstall::collect_uninstall("uninstall"));
        crate::log::info(&format!(
            "uninstall registry: +{} -> total {} in {:?}",
            raw.len() - before,
            raw.len(),
            t0.elapsed()
        ));
    }

    if pass.include_app_paths() && !budget_exhausted(budget, t0) && raw.len() < max_total {
        let t = Instant::now();
        registry::collect_app_paths("app-paths", &mut raw);
        crate::log::info(&format!(
            "app-paths: +... total {} in {:?}",
            raw.len(),
            t.elapsed()
        ));
    }

    // 仅 Full 裁剪 unseen：Bootstrap/Fast 预算/深度有限，不能清掉深层缓存。
    cache.save(matches!(pass, ScanPass::Full));
    crate::log::info(&format!(
        "lnk cache: {} hits / {} misses in {:?}",
        cache.hits,
        cache.misses,
        t0.elapsed()
    ));

    if let Some(handle) = uwp_worker {
        let t = Instant::now();
        match handle.join() {
            Ok(mut items) => {
                let before = raw.len();
                raw.append(&mut items);
                crate::log::info(&format!(
                    "apps-folder: +{} -> total {} in {:?} (joined)",
                    raw.len() - before,
                    raw.len(),
                    t.elapsed()
                ));
            }
            Err(_) => crate::log::info("uwp worker panicked; skip"),
        }
    }

    let mut items = dedupe(raw);
    merge_command_name_duplicates(&mut items);
    absorb_discovery_rows(&mut items);
    merge_same_name_formal_duplicates(&mut items);
    let system_entries = crate::app::builtin::materialize_system_entries(Some(icon_dir));
    exclude_system_name_duplicates(&mut items, &system_entries);
    crate::log::info(&format!(
        "dedupe/system filter -> {} in {:?}",
        items.len(),
        t0.elapsed()
    ));

    let mut apps = Vec::with_capacity(items.len());
    let t = Instant::now();
    let mut py = Duration::ZERO;
    let mut metadata_time = Duration::ZERO;
    let meta_batch: Vec<Vec<String>> = if pass.include_metadata() {
        let tm = Instant::now();
        // 命令别名以文件名精确检索；PATH 中的 CLI 数量可能很多，不为它们读取
        // 版本资源。正式应用仍保留版本关键词。
        let paths: Vec<(usize, PathBuf)> = items
            .iter()
            .enumerate()
            .filter(|(_, (item, _))| item.source != commands::COMMAND_SOURCE)
            .map(|(idx, (item, _))| (idx, PathBuf::from(&item.target)))
            .collect();
        let mut meta_cache = metadata::MetaCache::load(icon_dir);
        let found = metadata::executable_keywords_batch(
            &paths.iter().map(|(_, path)| path.clone()).collect::<Vec<_>>(),
            &mut meta_cache,
        );
        let mut batch = vec![Vec::new(); items.len()];
        for ((idx, _), keywords) in paths.into_iter().zip(found) {
            batch[idx] = keywords;
        }
        meta_cache.save();
        crate::log::info(&format!(
            "meta cache: {} hits / {} misses",
            meta_cache.hits, meta_cache.misses
        ));
        metadata_time += tm.elapsed();
        batch
    } else {
        Vec::new()
    };
    for (idx, (mut item, icon_src)) in items.into_iter().enumerate() {
        if pass.include_metadata() {
            if let Some(keywords) = meta_batch.get(idx) {
                for keyword in keywords {
                    if !item
                        .search_keywords
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(keyword))
                    {
                        item.search_keywords.push(keyword.clone());
                    }
                }
            }
        }
        let tp = Instant::now();
        item.attach_search_fields();
        py += tp.elapsed();
        item.icon_src = icon_src.or_else(|| Some(item.target.clone()));
        apps.push(item);
    }

    if pass.include_protocols() {
        let protocols = protocols::collect_protocols();
        let existing_len = apps.len();
        protocols::merge_or_materialize(&mut apps, &protocols);
        for item in &mut apps[existing_len..] {
            item.attach_search_fields();
            item.icon_src = Some(item.target.clone());
        }
    }

    let mut icon_time = Duration::ZERO;
    if pass.include_icons() {
        let t_icon = Instant::now();
        let pending: Vec<(String, Option<String>, Option<String>)> = apps
            .iter()
            .filter(|a| a.icon.is_none())
            .map(|a| {
                let src = a.icon_src.clone().filter(|s| !s.is_empty());
                let target = Some(a.target.clone()).filter(|s| !s.is_empty());
                (a.id.clone(), src, target)
            })
            .collect();
        let extracted = extract_icons_parallel(&pending, icon_dir);
        for app in apps.iter_mut() {
            if app.icon.is_none() {
                if let Some(path) = extracted.get(&app.id) {
                    app.icon = path.clone();
                }
            }
        }
        icon_time = t_icon.elapsed();
        crate::log::info(&format!(
            "icons parallel: {} pending in {icon_time:?}",
            pending.len()
        ));
    }
    crate::log::info(&format!(
        "fields pass={pass:?}: {} apps in {:?} metadata={metadata_time:?} pinyin={py:?} icons={icon_time:?}",
        apps.len(),
        t.elapsed()
    ));

    let t_idx = Instant::now();
    let mut index = AppIndex {
        apps,
        system_entries,
        retrieval: None,
    };
    index.rebuild_retrieval();
    crate::log::info(&format!(
        "retrieval index docs={} built in {:?}",
        index.apps.len() + index.system_entries.len(),
        t_idx.elapsed()
    ));
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kite-scanner-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scoop_shims_are_indexed() {
        let root = temp_dir("scoop");
        let shims = root.join("scoop").join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        let target = shims.join("ripgrep.exe");
        std::fs::write(&target, b"fixture").unwrap();
        let icon_dir = root.join("icons");
        std::fs::create_dir_all(&icon_dir).unwrap();

        let index = scan_apps_pass_with_options(
            &icon_dir,
            ScanPass::Fast,
            &ScanOptions {
                extra_scoop_shim_dirs: vec![shims],
                ..ScanOptions::default()
            },
        );

        assert!(
            index
                .apps
                .iter()
                .any(|item| item.target == target.to_string_lossy()),
            "Scoop shim 应进入应用索引"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_portable_directory_is_indexed() {
        let root = temp_dir("portable-root");
        let portable = root.join("My Portable Apps");
        std::fs::create_dir_all(&portable).unwrap();
        let exe = portable.join("Portable Editor.exe");
        std::fs::write(&exe, b"fixture").unwrap();
        let icon_dir = root.join("icons");
        // 这里只验证用户目录能进入检索索引；快扫时限由 pass 测试单独验证。
        let index = scan_apps_with_budget(
            &icon_dir,
            ScanPass::Fast,
            &ScanOptions {
                portable_dirs: vec![portable],
                ..ScanOptions::default()
            },
            None,
        );
        assert!(
            index
                .retrieval
                .as_ref()
                .unwrap()
                .search("Portable Editor", &[], 10)
                .iter()
                .any(|hit| hit.item.target == exe.to_string_lossy()),
            "an explicitly configured portable application must be searchable"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bootstrap_pass_skips_scoop_and_command_sources() {
        let root = temp_dir("bootstrap-skip");
        let shims = root.join("scoop").join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        let target = shims.join("ripgrep.exe");
        std::fs::write(&target, b"fixture").unwrap();
        let icon_dir = root.join("icons");
        std::fs::create_dir_all(&icon_dir).unwrap();

        let portable = root.join("portable");
        std::fs::create_dir_all(&portable).unwrap();
        let portable_exe = portable.join("Portable Tool.exe");
        std::fs::write(&portable_exe, b"fixture").unwrap();

        let options = ScanOptions {
            extra_scoop_shim_dirs: vec![shims.clone()],
            portable_dirs: vec![portable.clone()],
            force_uwp_refresh: false,
        };

        // 来源集合与时间预算是两个契约；本测试固定前者，避免主机负载影响收录。
        let bootstrap = scan_apps_with_budget(&icon_dir, ScanPass::Bootstrap, &options, None);
        assert!(
            !bootstrap
                .apps
                .iter()
                .any(|item| item.target == target.to_string_lossy()),
            "Bootstrap must not index Scoop shims"
        );
        assert!(
            !bootstrap
                .apps
                .iter()
                .any(|item| item.source == "commands"),
            "Bootstrap must not index command aliases"
        );
        assert!(
            bootstrap
                .apps
                .iter()
                .any(|item| item.source == "portable"
                    && item.target == portable_exe.to_string_lossy()),
            "Bootstrap must keep configured portable apps as formal entries"
        );
        assert!(
            bootstrap.retrieval.is_some(),
            "Bootstrap must publish a searchable retrieval index"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn full_pass_still_indexes_scoop_shims() {
        let root = temp_dir("full-scoop");
        let shims = root.join("scoop").join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        let target = shims.join("ripgrep.exe");
        std::fs::write(&target, b"fixture").unwrap();
        let icon_dir = root.join("icons");
        std::fs::create_dir_all(&icon_dir).unwrap();

        let full = scan_apps_pass_with_options(
            &icon_dir,
            ScanPass::Full,
            &ScanOptions {
                extra_scoop_shim_dirs: vec![shims],
                ..ScanOptions::default()
            },
        );
        assert!(
            full.apps
                .iter()
                .any(|item| item.target == target.to_string_lossy()),
            "Full pass must keep Scoop shims searchable"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
