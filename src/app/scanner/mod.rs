//! 扫描开始菜单 / 桌面 / Scoop shims，解析快捷方式，去重。
//! 注意：不要 follow_links（开始菜单 junction 可能卡死）；快速扫描有时间预算。

mod cache;
mod lnk;
mod registry;
pub mod util;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use walkdir::WalkDir;

use cache::ScanCache;
use crate::model::{AppIndex, AppItem};
use crate::system::icons;
use util::{app_display_name, hash_id, normalize_path_key};

type RawItem = (AppItem, Option<String>);

/// 快速扫描时间预算；超时后用已收集结果。
const FAST_BUDGET: Duration = Duration::from_millis(1500);
const MAX_PER_DIR: usize = 400;
const MAX_TOTAL: usize = 1500;
/// Start Menu 应用通常位于 Programs/<分类>/<应用>，开发工具还可能再嵌套一层。
/// 上限覆盖常见入口，同时继续受每目录、总量和快速扫描预算约束。
const START_MENU_MAX_DEPTH: usize = 5;
const OTHER_SOURCE_MAX_DEPTH: usize = 2;
/// 后台完整扫描：不受快扫时间预算限制，递归更深；安全上限触发时写日志。
const FULL_START_MENU_MAX_DEPTH: usize = 16;
const FULL_OTHER_SOURCE_MAX_DEPTH: usize = 8;
const FULL_MAX_PER_DIR: usize = 2000;
const FULL_MAX_TOTAL: usize = 8000;

/// 扫描档位：首屏快扫 vs 后台完整补扫。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPass {
    /// 有时间预算、浅层、不提图标、不含 UWP，尽早发布首屏索引。
    Fast,
    /// 无时间预算、深层递归、含 UWP 与图标，原子替换快照。
    Full,
}

/// 完整扫描。`fast` 为 true 时只建列表、不提图标（首屏用）。
pub fn scan_apps(icon_dir: &Path, fast: bool) -> AppIndex {
    let pass = if fast { ScanPass::Fast } else { ScanPass::Full };
    scan_apps_pass(icon_dir, pass, &[])
}

/// 按档位扫描；完整档可作为后台第二遍原子替换首屏快照。
pub fn scan_apps_pass(icon_dir: &Path, pass: ScanPass, extra_scoop_shim_dirs: &[PathBuf]) -> AppIndex {
    scan_apps_with_scoop_shims(icon_dir, pass, extra_scoop_shim_dirs)
}

fn scan_apps_with_scoop_shims(
    icon_dir: &Path,
    pass: ScanPass,
    extra_scoop_shim_dirs: &[PathBuf],
) -> AppIndex {
    let fast = pass == ScanPass::Fast;
    let t0 = Instant::now();
    let budget = if fast { Some(FAST_BUDGET) } else { None };
    let start_depth = if fast {
        START_MENU_MAX_DEPTH
    } else {
        FULL_START_MENU_MAX_DEPTH
    };
    let other_depth = if fast {
        OTHER_SOURCE_MAX_DEPTH
    } else {
        FULL_OTHER_SOURCE_MAX_DEPTH
    };
    let max_per_dir = if fast {
        MAX_PER_DIR
    } else {
        FULL_MAX_PER_DIR
    };
    let max_total = if fast { MAX_TOTAL } else { FULL_MAX_TOTAL };
    let mut raw: Vec<RawItem> = Vec::new();
    let mut cache = ScanCache::load(icon_dir);

    let user_start = dirs::data_dir()
        .map(|d| d.join("Microsoft/Windows/Start Menu"))
        .unwrap_or_default();
    let common_start = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu");
    let user_desktop = dirs::desktop_dir().unwrap_or_default();
    let public_desktop = PathBuf::from(r"C:\Users\Public\Desktop");
    let user_home = dirs::home_dir();
    let program_data = std::env::var_os("ProgramData").map(PathBuf::from);
    let scoop_root = std::env::var_os("SCOOP").map(PathBuf::from);
    let scoop_global_root = std::env::var_os("SCOOP_GLOBAL").map(PathBuf::from);

    crate::log::info(&format!("scan pass={pass:?} start"));

    // Scoop exposes installed applications through its shims directory rather
    // than Start Menu shortcuts. Scan this small, well-known directory first so
    // the fast pass keeps package-managed apps available even when the larger
    // Start Menu scan consumes its time budget.
    let before = raw.len();
    collect_scoop_shims(
        user_home.as_deref(),
        program_data.as_deref(),
        scoop_root.as_deref(),
        scoop_global_root.as_deref(),
        extra_scoop_shim_dirs,
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

    if !budget_exhausted(budget, t0) && raw.len() < max_total {
        let t = Instant::now();
        registry::collect_app_paths("app-paths", &mut raw);
        crate::log::info(&format!(
            "app-paths: +... total {} in {:?}",
            raw.len(),
            t.elapsed()
        ));
    }

    cache.save();
    crate::log::info(&format!(
        "lnk cache: {} hits / {} misses in {:?}",
        cache.hits,
        cache.misses,
        t0.elapsed()
    ));

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
            item.icon = icons::cache_icon(
                icon_dir,
                &item.id,
                item.icon_src.as_deref(),
                Some(item.target.as_str()),
            );
        }
        apps.push(item);
    }
    crate::log::info(&format!(
        "fields/icons pass={pass:?}: {} apps in {:?} pinyin={py:?}",
        apps.len(),
        t.elapsed()
    ));

    // 系统入口随快照物化并补齐图标；检索索引与 apps+system 同代发布
    let system_entries = crate::app::builtin::materialize_system_entries(Some(icon_dir));
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

fn budget_exhausted(budget: Option<Duration>, t0: Instant) -> bool {
    budget.is_some_and(|b| t0.elapsed() >= b)
}

/// 收集缺少图标的条目 (id, 首选源, target 回退)。调用方短暂持锁后即可释放。
pub fn missing_icon_targets(index: &AppIndex) -> Vec<(String, Option<String>, Option<String>)> {
    index
        .apps
        .iter()
        .filter(|a| a.icon.is_none())
        .map(|a| {
            let src = a.icon_src.clone().filter(|s| !s.is_empty());
            let target = Some(a.target.clone()).filter(|s| !s.is_empty());
            (a.id.clone(), src, target)
        })
        .collect()
}

/// 并行提取图标，返回 id → PNG 路径；提取过程不持任何锁，调用方拿结果后短暂持锁合并。
pub fn extract_icons_parallel(
    pending: &[(String, Option<String>, Option<String>)],
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
            let part: Vec<(String, Option<String>, Option<String>)> = part.to_vec();
            let icon_dir = icon_dir.to_path_buf();
            let results = Arc::clone(&results);
            s.spawn(move || {
                for (id, src, target) in &part {
                    let path = icons::cache_icon(
                        &icon_dir,
                        id,
                        src.as_deref(),
                        target.as_deref(),
                    );
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
                let parsed = lnk::resolve_lnk(path);
                if let Some(r) = &parsed {
                    cache.record(path, r);
                }
                parsed
            };
            if let Some((target, args, working_dir, icon_src)) = resolved {
                let name = app_display_name(&file_name);
                let id = util::stable_item_id(&target, args.as_deref());
                let item = AppItem::scanned(id, name, target, args, working_dir, source);
                out.push((item, icon_src));
                n += 1;
            }
        } else if ext == "exe" {
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = util::stable_item_id(&target, None);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let icon_src = Some(target.clone());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, icon_src));
            n += 1;
        }
    }
}

fn collect_scoop_shims(
    home: Option<&Path>,
    program_data: Option<&Path>,
    scoop_root: Option<&Path>,
    scoop_global_root: Option<&Path>,
    extra_shim_dirs: &[PathBuf],
    budget: Option<Duration>,
    t0: Instant,
    max_depth: usize,
    max_per_dir: usize,
    max_total: usize,
    out: &mut Vec<RawItem>,
    cache: &mut ScanCache,
) {
    let mut roots = extra_shim_dirs.to_vec();
    if let Some(home) = home {
        roots.push(home.join("scoop").join("shims"));
    }
    if let Some(root) = scoop_root {
        roots.push(root.join("shims"));
    }
    if let Some(root) = scoop_global_root {
        roots.push(root.join("shims"));
    }
    if let Some(program_data) = program_data {
        roots.push(program_data.join("scoop").join("shims"));
    }

    let mut seen = std::collections::HashSet::new();
    for root in roots {
        if !seen.insert(normalize_path_key(&root.to_string_lossy())) {
            continue;
        }
        collect_from_dir(
            &root,
            "scoop",
            max_depth,
            budget,
            t0,
            max_per_dir,
            max_total,
            out,
            cache,
        );
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
        let key = dedupe_key(&item);
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

/// 同一 target 的不同参数可能代表不同的启动语义（例如普通 PowerShell
/// 与 Developer PowerShell），不能仅按 exe 路径合并。工作目录不参与
/// 去重：开始菜单、桌面快捷方式经常只是在快捷方式元数据中提供了
/// 不同的起始位置，而启动器会将无效或未提供的目录统一回落到用户主目录。
fn dedupe_key(item: &AppItem) -> String {
    let target = normalize_path_key(&item.target);
    hash_id(&[&target, item.args.as_deref().unwrap_or("")])
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let index = scan_apps_with_scoop_shims(&icon_dir, ScanPass::Fast, &[shims]);

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
            MAX_PER_DIR,
            MAX_TOTAL,
            &mut raw,
            &mut cache,
        );

        let targets: Vec<_> = raw
            .iter()
            .map(|(item, _)| item.target.as_str())
            .collect();
        assert!(targets.contains(&first.to_string_lossy().as_ref()));
        assert!(targets.contains(&second.to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn full_pass_recurses_deeper_than_fast_depth() {
        // WalkDir depth: root=0. fast max_depth=5 可见 a/b/c/X.exe (depth 4)，
        // 看不到 a/b/c/d/e/Deep.exe (depth 6)。
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

    #[test]
    fn dedupe_preserves_distinct_launch_arguments() {
        let target = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        let normal = AppItem::scanned(
            "normal".into(),
            "PowerShell".into(),
            target.into(),
            None,
            Some(r"C:\Users\admin".into()),
            "start-menu",
        );
        let normal_duplicate = AppItem::scanned(
            "normal-duplicate".into(),
            "PowerShell (desktop)".into(),
            target.into(),
            None,
            Some(r"C:\Users\admin\Desktop".into()),
            "desktop",
        );
        let developer = AppItem::scanned(
            "developer".into(),
            "Developer PowerShell".into(),
            target.into(),
            Some("-NoExit -Command Enter-VsDevShell".into()),
            Some(r"C:\Users\admin".into()),
            "start-menu",
        );

        let items = dedupe(vec![(normal, None), (normal_duplicate, None), (developer, None)]);

        assert_eq!(items.len(), 2, "相同启动语义应合并，不同参数必须保留");
        assert!(items.iter().any(|(item, _)| item.name == "Developer PowerShell"));
    }
}
