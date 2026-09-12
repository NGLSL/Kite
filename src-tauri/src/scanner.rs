use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_SZ,
};

use crate::model::AppItem;

pub struct AppIndex {
    pub apps: Vec<AppItem>,
}

impl AppIndex {
    pub fn empty() -> Self {
        Self { apps: Vec::new() }
    }
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn hash_id(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\0");
    }
    h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn normalize_path_key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

fn app_display_name(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.to_string())
}

/// Resolve a .lnk to (target, args, working_dir, icon_path).
fn resolve_lnk(path: &Path) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
    let shortcut = lnk::ShellLink::open(path).ok()?;

    let target = shortcut
        .link_info()
        .as_ref()
        .and_then(|info| {
            info.local_base_path_unicode()
                .as_ref()
                .or_else(|| info.local_base_path().as_ref())
        })
        .cloned()
        .or_else(|| shortcut.relative_path().clone())?;

    let args = shortcut
        .arguments()
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let working_dir = shortcut
        .working_dir()
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let icon = shortcut
        .icon_location()
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // If relative path, try to resolve against the .lnk directory
    let target = if !Path::new(&target).is_absolute() {
        path.parent()
            .map(|parent| parent.join(&target))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(target)
    } else {
        target
    };

    Some((target, args, working_dir, icon))
}

fn is_skippable_shortcut(name: &str) -> bool {
    let lower = name.to_lowercase();
    const SKIP: &[&str] = &[
        "uninstall",
        "unins000",
        "help",
        "readme",
        "license",
        "documentation",
        "website",
        "release notes",
        "release notes",
        "setup",
    ];
    SKIP.iter().any(|s| lower.contains(s))
}

fn collect_from_dir(root: &Path, source: &str, out: &mut Vec<(AppItem, Option<String>)>) {
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
        if is_skippable_shortcut(&file_name) {
            continue;
        }

        if ext == "lnk" {
            if let Some((target, args, working_dir, icon_src)) = resolve_lnk(path) {
                let name = app_display_name(&file_name);
                let item = AppItem {
                    id: hash_id(&[
                        &normalize_path_key(&target),
                        args.as_deref().unwrap_or(""),
                        source,
                    ]),
                    name: name.clone(),
                    display_name: name,
                    target,
                    args,
                    working_dir,
                    icon: None,
                    source: source.to_string(),
                };
                out.push((item, icon_src));
            }
        } else if ext == "exe" {
            let target = path.to_string_lossy().to_string();
            let name = app_display_name(&file_name);
            let item = AppItem {
                id: hash_id(&[&normalize_path_key(&target), source]),
                name: name.clone(),
                display_name: name,
                target: target.clone(),
                args: None,
                working_dir: path.parent().map(|p| p.to_string_lossy().to_string()),
                icon: None,
                source: source.to_string(),
            };
            out.push((item, Some(target)));
        }
    }
}

fn collect_app_paths(source: &str, out: &mut Vec<(AppItem, Option<String>)>) {
    let roots = [
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
        ),
        (
            HKEY_CURRENT_USER,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
        ),
    ];

    for (hive, subkey) in roots {
        let subkey_wide = wide(subkey);
        let mut hkey = Default::default();
        unsafe {
            if RegOpenKeyExW(hive, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut hkey)
                .is_err()
            {
                continue;
            }
        }
        let mut index = 0u32;
        loop {
            let mut name_buf = [0u16; 256];
            let mut name_len = name_buf.len() as u32;
            let enum_ok = unsafe {
                RegEnumKeyExW(
                    hkey,
                    index,
                    Some(PWSTR(name_buf.as_mut_ptr())),
                    &mut name_len,
                    None,
                    None,
                    None,
                    None,
                )
                .is_ok()
            };
            if !enum_ok {
                break;
            }
            let sub = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let sub_full = format!("{subkey}\\{sub}");
            let sub_full_wide = wide(&sub_full);
            let mut child = Default::default();
            if unsafe {
                RegOpenKeyExW(
                    hive,
                    PCWSTR(sub_full_wide.as_ptr()),
                    Some(0),
                    KEY_READ,
                    &mut child,
                )
            }
            .is_ok()
            {
                let mut data = vec![0u8; 1024];
                let mut data_len = data.len() as u32;
                let mut ty = REG_SZ;
                let empty = wide("");
                let q = unsafe {
                    RegQueryValueExW(
                        child,
                        PCWSTR(empty.as_ptr()),
                        None,
                        Some(&mut ty),
                        Some(data.as_mut_ptr()),
                        Some(&mut data_len),
                    )
                    .is_ok()
                };
                if q {
                    let target = u16_slice_to_string(&data, data_len);
                    let target = target.trim_matches('\0').trim().to_string();
                    if !target.is_empty() {
                        let name = Path::new(&target)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| sub.trim_end_matches(".exe").to_string());
                        let item = AppItem {
                            id: hash_id(&[&normalize_path_key(&target), source]),
                            name: name.clone(),
                            display_name: name,
                            target: target.clone(),
                            args: None,
                            working_dir: Path::new(&target)
                                .parent()
                                .map(|p| p.to_string_lossy().to_string()),
                            icon: None,
                            source: source.to_string(),
                        };
                        out.push((item, Some(target)));
                    }
                }
                unsafe {
                    let _ = RegCloseKey(child);
                }
            }
            index += 1;
        }
        unsafe {
            let _ = RegCloseKey(hkey);
        }
    }
}

fn u16_slice_to_string(data: &[u8], len: u32) -> String {
    let bytes = &data[..len as usize];
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        u16s.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    while u16s.last() == Some(&0) {
        u16s.pop();
    }
    String::from_utf16_lossy(&u16s)
}

fn dedupe(raw: Vec<(AppItem, Option<String>)>) -> Vec<(AppItem, Option<String>)> {
    let rank = |source: &str| match source {
        "start-menu" => 0,
        "desktop" => 1,
        "app-paths" => 2,
        _ => 3,
    };

    let mut best: HashMap<String, (AppItem, Option<String>)> = HashMap::new();
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

pub fn scan_apps(icon_dir: &Path) -> AppIndex {
    let mut raw: Vec<(AppItem, Option<String>)> = Vec::new();

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
    collect_app_paths("app-paths", &mut raw);

    let items = dedupe(raw);
    let mut apps = Vec::with_capacity(items.len());
    for (mut item, icon_src) in items {
        item.icon = crate::icons::cache_icon(icon_dir, &item.id, icon_src.as_deref());
        apps.push(item);
    }

    AppIndex { apps }
}
