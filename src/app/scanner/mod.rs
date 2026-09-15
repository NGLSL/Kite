//! 扫描开始菜单 / 桌面 / Scoop shims，解析快捷方式，去重。
//! 注意：不要 follow_links（开始菜单 junction 可能卡死）；快速扫描有时间预算。

mod cache;
mod commands;
mod lnk;
mod metadata;
mod protocols;
mod registry;
mod uninstall;
mod url;
pub mod util;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use walkdir::WalkDir;

use crate::model::{AppIndex, AppItem};
use crate::system::icons;
use cache::{ScanCache, UwpCache};
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

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub extra_scoop_shim_dirs: Vec<PathBuf>,
    pub portable_dirs: Vec<PathBuf>,
    /// 设置/托盘「重新扫描」：强制重枚举 UWP，忽略结果缓存 TTL。
    pub force_uwp_refresh: bool,
}

/// 完整扫描。`fast` 为 true 时只建列表、不提图标（首屏用）。
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

/// Fixed command-source directories that should participate in filesystem
/// change monitoring together with Start Menu and configured portable roots.
pub fn command_watch_roots() -> Vec<PathBuf> {
    commands::configured_command_roots()
}

fn scan_apps_with_options(icon_dir: &Path, pass: ScanPass, options: &ScanOptions) -> AppIndex {
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
    let max_per_dir = if fast { MAX_PER_DIR } else { FULL_MAX_PER_DIR };
    let max_total = if fast { MAX_TOTAL } else { FULL_MAX_TOTAL };
    let mut raw: Vec<RawItem> = Vec::new();
    let mut cache = ScanCache::load(icon_dir);

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
    // 结果落盘缓存：TTL 内自动重建直接复用；force_uwp_refresh 强制重枚举。
    let uwp_force = options.force_uwp_refresh;
    let uwp_icon_dir = icon_dir.to_path_buf();
    let uwp_worker = if fast {
        None
    } else {
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
                        let mut item = crate::model::AppItem::scanned(
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
    };

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

    // User-authorized roots take priority over broad system sources so the
    // fast-pass budget cannot starve explicitly configured portable apps.
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

    // Package managers and Windows app aliases expose small, fixed command
    // directories. They are intentionally scanned directly instead of walking
    // the complete PATH, which would pull in SDK and runtime internals.
    let before = raw.len();
    commands::collect_command_entries(commands::COMMAND_SOURCE, &mut raw);
    crate::log::info(&format!(
        "command aliases: +{} -> total {} in {:?}",
        raw.len() - before,
        raw.len(),
        t0.elapsed()
    ));

    // Installed programs without Start Menu entries often still publish a
    // friendly DisplayName and executable icon through Uninstall registry rows.
    let before = raw.len();
    raw.extend(uninstall::collect_uninstall("uninstall"));
    crate::log::info(&format!(
        "uninstall registry: +{} -> total {} in {:?}",
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

    // AppsFolder 枚举较慢；Full 档已在扫描开始时后台启动。
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
    // Full：先并行取版本资源关键词（缓存 + 线程池），再逐条合并，避免串行 ~50ms/文件。
    let meta_batch: Vec<Vec<String>> = if fast {
        Vec::new()
    } else {
        let tm = Instant::now();
        let paths: Vec<PathBuf> = items
            .iter()
            .map(|(item, _)| PathBuf::from(&item.target))
            .collect();
        let mut meta_cache = metadata::MetaCache::load(icon_dir);
        let batch = metadata::executable_keywords_batch(&paths, &mut meta_cache);
        meta_cache.save();
        crate::log::info(&format!(
            "meta cache: {} hits / {} misses",
            meta_cache.hits, meta_cache.misses
        ));
        metadata_time += tm.elapsed();
        batch
    };
    for (idx, (mut item, icon_src)) in items.into_iter().enumerate() {
        if !fast {
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
        // 保留提取源；Full 档在列表收齐后并行提图标，避免串行 Shell 提取。
        item.icon_src = icon_src.or_else(|| Some(item.target.clone()));
        apps.push(item);
    }

    // Protocol schemes are aliases for an existing executable whenever
    // possible. Only registrations with no indexed target materialize a new
    // row, and those still launch the verified executable rather than a URI.
    let protocols = protocols::collect_protocols();
    let existing_len = apps.len();
    protocols::merge_or_materialize(&mut apps, &protocols);
    for item in &mut apps[existing_len..] {
        item.attach_search_fields();
        item.icon_src = Some(item.target.clone());
    }

    let mut icon_time = Duration::ZERO;
    if !fast {
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

    // 系统入口随快照物化并补齐图标；检索索引与 apps+system 同代发布
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
                    let path = icons::cache_icon(&icon_dir, id, src.as_deref(), target.as_deref());
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
            let id = util::stable_item_id(&target, None);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, None));
            n += 1;
        } else if ext == "appref-ms" {
            // ClickOnce application references are themselves verified files;
            // Windows Shell resolves deployment metadata when the user opens it.
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let id = util::stable_item_id(&target, None);
            let working_dir = path.parent().map(|p| p.to_string_lossy().to_string());
            let item = AppItem::scanned(id, name, target, None, working_dir, source);
            out.push((item, None));
            n += 1;
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
    use std::collections::hash_map::Entry;

    let rank = |source: &str| match source {
        "start-menu" => 0,
        "desktop" => 1,
        "portable" => 2,
        "uninstall" => 3,
        "app-paths" => 4,
        "uwp" | "apps-folder" => 5,
        "commands" => 6,
        _ => 7,
    };

    let mut best: HashMap<String, RawItem> = HashMap::new();
    for (item, icon) in raw {
        let key = dedupe_key(&item);
        match best.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert((item, icon));
            }
            Entry::Occupied(mut slot) => {
                let current_rank = rank(&slot.get().0.source);
                let incoming_rank = rank(&item.source);
                let replace = incoming_rank < current_rank
                    || (incoming_rank == current_rank
                        && deterministic_item_key(&item) < deterministic_item_key(&slot.get().0));
                if !replace {
                    keep_shortcut_names(&mut slot.get_mut().0, &item);
                } else {
                    let (old_item, _) = slot.get();
                    let mut replacement = item;
                    keep_shortcut_names(&mut replacement, old_item);
                    slot.insert((replacement, icon));
                }
            }
        }
    }

    let mut list: Vec<_> = best.into_values().collect();
    list.sort_by(|a, b| {
        a.0.name
            .to_lowercase()
            .cmp(&b.0.name.to_lowercase())
            .then_with(|| a.0.source.cmp(&b.0.source))
            .then_with(|| normalize_path_key(&a.0.target).cmp(&normalize_path_key(&b.0.target)))
            .then_with(|| a.0.args.cmp(&b.0.args))
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    list
}

fn deterministic_item_key(item: &AppItem) -> (String, String, String, String, String) {
    (
        item.name.to_lowercase(),
        item.display_name.to_lowercase(),
        normalize_path_key(&item.target),
        item.args.clone().unwrap_or_default(),
        item.id.clone(),
    )
}

fn exclude_system_name_duplicates(apps: &mut Vec<RawItem>, system_entries: &[AppItem]) {
    let system_names: std::collections::HashSet<String> = system_entries
        .iter()
        .map(|item| crate::search::normalize_for_index(&item.name))
        .collect();
    apps.retain(|(item, _)| {
        item.source != "apps-folder"
            || !system_names.contains(&crate::search::normalize_for_index(&item.name))
    });
}

/// WindowsApps often exposes `MediaPlayer.exe` beside an AppsFolder row named
/// `Media Player`. Preserve the executable alias as a keyword on the friendly
/// row instead of showing two visually equivalent applications.
fn merge_command_name_duplicates(items: &mut Vec<RawItem>) {
    let compact_name = |item: &AppItem| {
        crate::search::normalize_for_index(&item.name)
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
    };
    let mut remove = std::collections::HashSet::new();
    for index in 0..items.len() {
        if items[index].0.source != commands::COMMAND_SOURCE {
            continue;
        }
        let key = compact_name(&items[index].0);
        let Some(winner) = items.iter().enumerate().find_map(|(candidate, (item, _))| {
            (candidate != index
                && item.source != commands::COMMAND_SOURCE
                && compact_name(item) == key)
                .then_some(candidate)
        }) else {
            continue;
        };
        let alias = items[index].0.clone();
        keep_shortcut_names(&mut items[winner].0, &alias);
        remove.insert(index);
    }
    if !remove.is_empty() {
        let mut index = 0usize;
        items.retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }
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
    fn dedupe_winner_is_independent_of_source_enumeration_order() {
        let make = |name: &str| {
            AppItem::scanned(
                "same-id".to_string(),
                name.to_string(),
                r"C:\Apps\same.exe".to_string(),
                None,
                None,
                "start-menu",
            )
        };
        let forward = dedupe(vec![(make("Zeta"), None), (make("Alpha"), None)]);
        let reverse = dedupe(vec![(make("Alpha"), None), (make("Zeta"), None)]);

        assert_eq!(forward[0].0.name, reverse[0].0.name);
        assert_eq!(forward[0].0.name, "Alpha");
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

        let targets: Vec<_> = raw.iter().map(|(item, _)| item.target.as_str()).collect();
        assert!(targets.contains(&first.to_string_lossy().as_ref()));
        assert!(targets.contains(&second.to_string_lossy().as_ref()));
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
        let index = scan_apps_pass_with_options(
            &icon_dir,
            ScanPass::Fast,
            &ScanOptions {
                portable_dirs: vec![portable],
                ..ScanOptions::default()
            },
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
            MAX_PER_DIR,
            MAX_TOTAL,
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
            MAX_PER_DIR,
            MAX_TOTAL,
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

        let items = dedupe(vec![
            (normal, None),
            (normal_duplicate, None),
            (developer, None),
        ]);

        assert_eq!(items.len(), 2, "相同启动语义应合并，不同参数必须保留");
        assert!(items
            .iter()
            .any(|(item, _)| item.name == "Developer PowerShell"));
    }

    #[test]
    fn duplicate_shortcut_names_remain_searchable() {
        let target = r"C:\Program Files\Example\example.exe";
        let primary = AppItem::scanned(
            "same-id".into(),
            "Primary Launcher".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        let alternate = AppItem::scanned(
            "same-id".into(),
            "Alternate Console".into(),
            target.into(),
            None,
            None,
            "desktop",
        );
        for raw in [
            vec![(primary.clone(), None), (alternate.clone(), None)],
            vec![(alternate.clone(), None), (primary.clone(), None)],
        ] {
            let mut items = dedupe(raw);
            assert_eq!(items.len(), 1);
            let mut item = items.pop().unwrap().0;
            item.attach_search_fields();
            let index = crate::search::RetrievalIndex::build(&[item], &[]);
            assert!(
                index
                    .search("Alternate Console", &[], 10)
                    .iter()
                    .any(|hit| hit.item.name == "Primary Launcher"),
                "another installed shortcut name must still find the same target"
            );
        }
    }

    #[test]
    fn compact_command_alias_merges_into_friendly_application() {
        let friendly = AppItem::scanned(
            "friendly".into(),
            "Media Player".into(),
            r"shell:AppsFolder\MediaPlayer".into(),
            None,
            None,
            "uwp",
        );
        let command = AppItem::scanned(
            "command".into(),
            "MediaPlayer".into(),
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\MediaPlayer.exe".into(),
            None,
            None,
            commands::COMMAND_SOURCE,
        );
        let mut items = vec![(friendly, None), (command, None)];
        merge_command_name_duplicates(&mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0.name, "Media Player");
        assert!(items[0]
            .0
            .search_keywords
            .iter()
            .any(|keyword| keyword == "MediaPlayer"));
    }

    #[test]
    fn uninstall_display_name_wins_and_executable_alias_remains_searchable() {
        let target = r"C:\Apps\Editor\editor.exe";
        let app_path = AppItem::scanned(
            "app-path".into(),
            "editor".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        let uninstall = AppItem::scanned(
            "uninstall".into(),
            "Acme Document Editor".into(),
            target.into(),
            None,
            None,
            "uninstall",
        );
        let mut items = dedupe(vec![(app_path, None), (uninstall, None)]);
        assert_eq!(items.len(), 1);
        let mut item = items.pop().unwrap().0;
        assert_eq!(item.name, "Acme Document Editor");
        item.attach_search_fields();
        let index = crate::search::RetrievalIndex::build(&[item], &[]);
        for query in ["acme", "editor"] {
            assert_eq!(index.search(query, &[], 10).len(), 1);
        }
    }

    #[test]
    fn apps_folder_classic_does_not_duplicate_builtin_system_entry() {
        let app = AppItem::scanned(
            "apps-folder-control-panel".into(),
            "控制面板".into(),
            "shell:AppsFolder\\Microsoft.Windows.ControlPanel".into(),
            None,
            None,
            "apps-folder",
        );
        let builtin = AppItem::scanned(
            "builtin-control-panel".into(),
            "控制面板".into(),
            "shell:ControlPanelFolder".into(),
            None,
            None,
            "builtin-system",
        );
        let mut apps = vec![(app, None)];
        exclude_system_name_duplicates(&mut apps, &[builtin]);
        assert!(
            apps.is_empty(),
            "same-named AppsFolder row duplicates an existing system entry"
        );
    }
}

fn keep_shortcut_names(winner: &mut AppItem, other: &AppItem) {
    for name in std::iter::once(&other.name)
        .chain(std::iter::once(&other.display_name))
        .chain(other.search_keywords.iter())
    {
        if !name.eq_ignore_ascii_case(&winner.name)
            && !name.eq_ignore_ascii_case(&winner.display_name)
            && !winner
                .search_keywords
                .iter()
                .any(|kw| kw.eq_ignore_ascii_case(name))
        {
            winner.search_keywords.push(name.clone());
        }
    }
}
