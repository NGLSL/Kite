//! 入口变化监听：开始菜单 / 桌面 / 命令目录，以及应用注册表来源。
//! 通知 debounce 后合并为一次索引重建；监听失败不影响托盘手动重扫。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use windows::Win32::System::Registry::{
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_READ, KEY_WOW64_32KEY, REG_NOTIFY_CHANGE_LAST_SET,
    REG_NOTIFY_CHANGE_NAME,
};

/// 安装器会连写多个文件：安静期后触发，最长等待封顶避免一直拖着。
const QUIET: Duration = Duration::from_millis(400);
const MAX_WAIT: Duration = Duration::from_millis(2500);

/// 需要监听的目录集合。
pub fn watch_roots() -> Vec<PathBuf> {
    watch_roots_with_options(&crate::app::scanner::ScanOptions::default())
}

pub fn watch_roots_with_options(options: &crate::app::scanner::ScanOptions) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(start) = crate::app::scanner::util::user_start_menu_dir() {
        roots.push(start);
    } else if let Some(d) = dirs::data_dir() {
        roots.push(d.join("Microsoft/Windows/Start Menu"));
    }
    if let Some(start) = crate::app::scanner::util::common_start_menu_dir() {
        roots.push(start);
    } else {
        roots.push(PathBuf::from(
            r"C:\ProgramData\Microsoft\Windows\Start Menu",
        ));
    }
    if let Some(d) = dirs::desktop_dir() {
        roots.push(d);
    }
    roots.push(PathBuf::from(r"C:\Users\Public\Desktop"));
    for root in [
        std::env::var_os("SCOOP").map(PathBuf::from),
        std::env::var_os("SCOOP_GLOBAL").map(PathBuf::from),
        dirs::home_dir().map(|h| h.join("scoop")),
        std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("scoop")),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(root.join("shims"));
    }
    roots.extend(options.portable_dirs.iter().cloned());
    roots.extend(manual_watch_roots(options));
    roots.extend(crate::app::scanner::command_watch_roots());
    roots.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    roots.dedup_by(|a, b| {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    });
    roots
}

/// Parent directories of manual entries are watched so an external `.lnk`
/// retarget triggers the existing debounced rebuild.  Keep these roots
/// non-recursive: the scanner already has the persisted entry and does not
/// need to turn an arbitrary user directory into a new scan root.
fn manual_watch_roots(options: &crate::app::scanner::ScanOptions) -> Vec<PathBuf> {
    options
        .manual_apps
        .iter()
        .filter_map(|manual| {
            crate::app::actions::direct_path_candidate(&manual.path)
                .and_then(|path| path.parent().map(Path::to_path_buf))
        })
        .collect()
}

fn watch_mode_for_root(
    root: &Path,
    command_roots: &[PathBuf],
    manual_roots: &[PathBuf],
    existing_roots: &[PathBuf],
) -> RecursiveMode {
    let same_root = |candidate: &Path| {
        candidate
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(root.to_string_lossy().trim_end_matches(['\\', '/']))
    };
    let is_command_root = command_roots.iter().any(|candidate| same_root(candidate));
    let is_manual_only_root = manual_roots.iter().any(|candidate| same_root(candidate))
        && !existing_roots.iter().any(|candidate| same_root(candidate));
    if is_command_root || is_manual_only_root {
        RecursiveMode::NonRecursive
    } else {
        RecursiveMode::Recursive
    }
}

fn desired_watch_modes(
    options: &crate::app::scanner::ScanOptions,
) -> HashMap<PathBuf, RecursiveMode> {
    let command_roots = crate::app::scanner::command_scan_roots();
    let manual_roots = manual_watch_roots(options);
    let existing_options = crate::app::scanner::ScanOptions {
        manual_apps: Vec::new(),
        ..options.clone()
    };
    let existing_roots = watch_roots_with_options(&existing_options);
    watch_roots_with_options(options)
        .into_iter()
        .map(|root| {
            let mode = watch_mode_for_root(
                &root,
                &command_roots,
                &manual_roots,
                &existing_roots,
            );
            (root, mode)
        })
        .collect()
}

fn watch_mode_updates(
    watched_modes: &HashMap<PathBuf, RecursiveMode>,
    desired_modes: &HashMap<PathBuf, RecursiveMode>,
) -> Vec<(PathBuf, RecursiveMode)> {
    desired_modes
        .iter()
        .filter_map(|(root, desired)| {
            watched_modes
                .get(root)
                .filter(|current| *current != desired)
                .map(|_| (root.clone(), desired.clone()))
        })
        .collect()
}

/// 启动入口监听。`on_change` 在 debounce 合并后调用（通常触发 request_build）。
pub fn spawn_entry_watchers(
    scan_options: Arc<RwLock<crate::app::scanner::ScanOptions>>,
    on_change: impl Fn() + Send + 'static,
) {
    let dirty = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<()>();

    // 文件系统
    let fs_tx = tx.clone();
    let fs_dirty = Arc::clone(&dirty);
    std::thread::spawn(move || {
        let mut watcher = match RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if res.is_ok() {
                    fs_dirty.store(true, Ordering::SeqCst);
                    let _ = fs_tx.send(());
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                crate::log::info(&format!("fs watcher create failed: {e}"));
                return;
            }
        };
        let mut watched = std::collections::HashSet::new();
        let mut watched_modes = HashMap::new();
        loop {
            let options = scan_options
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let desired_modes: HashMap<PathBuf, RecursiveMode> = desired_watch_modes(&options)
                .into_iter()
                .filter(|(root, _)| root.exists())
                .collect();
            let desired: std::collections::HashSet<PathBuf> =
                desired_modes.keys().cloned().collect();

            let removed: Vec<PathBuf> = watched.difference(&desired).cloned().collect();
            for root in removed {
                let _ = watcher.unwatch(&root);
                watched.remove(&root);
                watched_modes.remove(&root);
            }

            let additions: Vec<PathBuf> = desired.difference(&watched).cloned().collect();
            for root in additions {
                // 命令目录只扫描直接子项；递归监听 PATH 下的工具目录会被构建输出频繁触发。
                // Codex bin 父目录仍递归监听，以捕获新版本目录。
                let mode = desired_modes[&root].clone();
                if let Err(e) = watcher.watch(&root, mode.clone()) {
                    crate::log::info(&format!("watch {:?} failed: {e}", root));
                } else {
                    watched.insert(root.clone());
                    watched_modes.insert(root.clone(), mode);
                    crate::log::info(&format!("watching {:?}", root));
                }
            }

            for (root, mode) in watch_mode_updates(&watched_modes, &desired_modes) {
                if let Err(error) = watcher.unwatch(&root) {
                    crate::log::info(&format!("unwatch {:?} for mode update failed: {error}", root));
                    continue;
                }
                watched_modes.remove(&root);
                if let Err(error) = watcher.watch(&root, mode.clone()) {
                    watched.remove(&root);
                    crate::log::info(&format!("rewatch {:?} for mode update failed: {error}", root));
                } else {
                    watched_modes.insert(root, mode);
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    // App Paths 注册表（HKLM + HKCU，含 32 位视图）
    let reg_tx = tx.clone();
    let reg_dirty = Arc::clone(&dirty);
    std::thread::spawn(move || {
        spawn_registry_watchers(reg_tx, reg_dirty);
    });

    // Debounce 聚合线程
    std::thread::spawn(move || {
        let mut first: Option<Instant> = None;
        loop {
            match rx.recv_timeout(QUIET) {
                Ok(()) => {
                    if first.is_none() {
                        first = Some(Instant::now());
                    }
                    // 继续吸收突发，直到安静期或封顶
                    loop {
                        let elapsed = first.map(|t| t.elapsed()).unwrap_or_default();
                        if elapsed >= MAX_WAIT {
                            break;
                        }
                        match rx.recv_timeout(QUIET) {
                            Ok(()) => {}
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if dirty.swap(false, Ordering::SeqCst) {
                        crate::log::info("entry watch: merged change -> rebuild");
                        on_change();
                    }
                    first = None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if dirty.swap(false, Ordering::SeqCst) {
                        crate::log::info("entry watch: dirty timeout -> rebuild");
                        on_change();
                    }
                    first = None;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn spawn_registry_watchers(tx: mpsc::Sender<()>, dirty: Arc<AtomicBool>) {
    for (hive_kind, subkey, wow) in registry_watch_specs() {
        let tx = tx.clone();
        let dirty = Arc::clone(&dirty);
        let subkey = subkey.to_string();
        std::thread::spawn(move || {
            let hive = if hive_kind == 0 {
                HKEY_LOCAL_MACHINE
            } else {
                HKEY_CURRENT_USER
            };
            watch_registry_key(hive, &subkey, wow, tx, dirty);
        });
    }
}

fn registry_watch_specs() -> Vec<(u8, &'static str, bool)> {
    let mut specs = Vec::new();
    specs.push((0, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment", false));
    specs.push((1, "Environment", false));
    let app_paths = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    specs.extend([
        (0, app_paths, false),
        (0, app_paths, true),
        (1, app_paths, false),
    ]);

    let uninstall = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    for hive in [0, 1] {
        specs.push((hive, uninstall, false));
        specs.push((hive, uninstall, true));
    }

    // 不递归监听完整 Software\Classes。该树包含大量与应用入口无关的
    // COM、文件关联和临时状态写入，任何一次写入都会造成完整索引重建。
    // 协议别名仍在 Full 扫描中读取；应用安装通常同时更新开始菜单、
    // Uninstall 或 App Paths，也可以由设置页手动重新扫描。
    specs
}

fn watch_registry_key(
    hive: HKEY,
    subkey: &str,
    wow64: bool,
    tx: mpsc::Sender<()>,
    dirty: Arc<AtomicBool>,
) {
    use crate::app::scanner::util::wide;
    let wide_sub = wide(subkey);
    let access = if wow64 {
        KEY_READ | KEY_NOTIFY | KEY_WOW64_32KEY
    } else {
        KEY_READ | KEY_NOTIFY
    };
    unsafe {
        let mut hkey = Default::default();
        let open = RegOpenKeyExW(
            hive,
            windows::core::PCWSTR(wide_sub.as_ptr()),
            None,
            access,
            &mut hkey,
        );
        if open.is_err() {
            crate::log::info(&format!("registry watch open failed: {subkey}"));
            return;
        }
        loop {
            // 同步等待：变更发生后返回，再 arm 下一轮
            let notified = RegNotifyChangeKeyValue(
                hkey,
                true,
                REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET,
                None,
                false,
            );
            if notified.is_err() {
                let _ = RegCloseKey(hkey);
                return;
            }
            dirty.store(true, Ordering::SeqCst);
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_roots_include_start_menu_and_desktop() {
        let roots = watch_roots();
        assert!(roots
            .iter()
            .any(|p| p.to_string_lossy().contains("Start Menu")));
        assert!(roots.iter().any(|p| {
            let s = p.to_string_lossy().to_lowercase();
            s.contains("desktop")
        }));
    }

    #[test]
    fn watch_roots_include_configured_portable_directories() {
        let root = PathBuf::from(r"D:\Portable Apps");
        let roots = watch_roots_with_options(&crate::app::scanner::ScanOptions {
            portable_dirs: vec![root.clone()],
            ..crate::app::scanner::ScanOptions::default()
        });
        assert!(roots.contains(&root));
    }

    #[test]
    fn watch_roots_include_manual_parent_without_widening_to_entry_path() {
        let root = PathBuf::from(r"D:\Registered Apps");
        let entry = root.join("Editor.lnk");
        let roots = watch_roots_with_options(&crate::app::scanner::ScanOptions {
            manual_apps: vec![crate::storage::ManualApp {
                id: 1,
                path: entry.to_string_lossy().into_owned(),
                display_name: "Editor".into(),
                item_id: "editor-id".into(),
            }],
            ..crate::app::scanner::ScanOptions::default()
        });
        assert!(roots.contains(&root));
        assert!(!roots.contains(&entry));

        let manual_roots = manual_watch_roots(&crate::app::scanner::ScanOptions {
            manual_apps: vec![crate::storage::ManualApp {
                id: 1,
                path: entry.to_string_lossy().into_owned(),
                display_name: "Editor".into(),
                item_id: "editor-id".into(),
            }],
            ..crate::app::scanner::ScanOptions::default()
        });
        assert_eq!(manual_roots, vec![root.clone()]);
        assert!(matches!(
            watch_mode_for_root(&root, &[], &manual_roots, &[]),
            RecursiveMode::NonRecursive
        ));
    }

    #[test]
    fn overlapping_existing_root_stays_recursive_when_manually_registered() {
        let root = PathBuf::from(r"D:\Registered Apps");
        let manual_roots = vec![root.clone()];
        let existing_roots = vec![root.clone()];
        assert!(matches!(
            watch_mode_for_root(&root, &[], &manual_roots, &existing_roots),
            RecursiveMode::Recursive
        ));
    }

    #[test]
    fn mode_coordinator_rewatches_manual_root_when_portable_is_added() {
        let root = PathBuf::from(r"D:\Kite Manual Dynamic");
        let entry = root.join("Editor.exe");
        let manual = crate::storage::ManualApp {
            id: 1,
            path: entry.to_string_lossy().into_owned(),
            display_name: "Editor".into(),
            item_id: "editor-id".into(),
        };
        let manual_only = crate::app::scanner::ScanOptions {
            manual_apps: vec![manual.clone()],
            ..crate::app::scanner::ScanOptions::default()
        };
        let manual_and_portable = crate::app::scanner::ScanOptions {
            manual_apps: vec![manual],
            portable_dirs: vec![root.clone()],
            ..crate::app::scanner::ScanOptions::default()
        };

        let before = desired_watch_modes(&manual_only);
        let after = desired_watch_modes(&manual_and_portable);
        assert_eq!(before.get(&root), Some(&RecursiveMode::NonRecursive));
        assert_eq!(after.get(&root), Some(&RecursiveMode::Recursive));

        // This is the same mode state held by spawn_entry_watchers. The
        // coordinator must schedule an unwatch/rewatch when the root remains
        // in the desired path set but its mode changes.
        let mut watched_modes = HashMap::from([(root.clone(), before[&root].clone())]);
        let updates = watch_mode_updates(&watched_modes, &after);
        assert_eq!(updates, vec![(root.clone(), RecursiveMode::Recursive)]);
        for (path, mode) in updates {
            watched_modes.insert(path, mode);
        }
        assert_eq!(
            watched_modes.get(&root),
            Some(&RecursiveMode::Recursive)
        );
    }

    #[test]
    fn registry_watch_specs_exclude_the_noisy_classes_tree() {
        let specs = registry_watch_specs();
        assert!(specs.iter().any(|(hive, key, _)| *hive == 1 && *key == "Environment"));
        assert!(specs.iter().any(|(hive, key, _)| {
            *hive == 0 && key.ends_with(r"Session Manager\Environment")
        }));
        for expected in ["App Paths", "Uninstall"] {
            assert!(
                specs.iter().any(|(_, key, _)| key.ends_with(expected)),
                "missing registry watcher for {expected}"
            );
        }
        assert!(specs
            .iter()
            .any(|(_, key, wow)| key.ends_with("Uninstall") && *wow));
        assert!(
            specs.iter().all(|(_, key, _)| key != &r"SOFTWARE\Classes"),
            "watching the complete Classes tree turns unrelated registry writes into rebuilds"
        );
    }
}
