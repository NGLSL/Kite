//! Control Panel 的系统入口枚举与启动目标解析。
//!
//! Windows 的「所有控制面板项」是一个 Shell 虚拟文件夹。这里从该文件夹
//! 枚举当前可见项，使用 Shell 返回的本地化名称和绝对解析路径；注册表只
//! 用来补充 CLSID 的显式 Open\\Command 与图标元数据。没有显式命令的项目
//! 只有在 Shell 能解析其目标后，才以 `shell:` target 进入应用索引。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ, REG_EXPAND_SZ,
    REG_SZ,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SHLoadIndirectString,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};

use crate::model::AppItem;

const CONTROL_PANEL_FOLDER: &str = "shell:ControlPanelFolder";

#[derive(Debug, Clone)]
struct ShellEntry {
    name: String,
    parsing_name: String,
    guid: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ControlPanelMetadata {
    guid: Option<String>,
    fallback_name: Option<String>,
    default_name: Option<String>,
    application_name: Option<String>,
    target: Option<String>,
    args: Option<String>,
    icon_src: Option<String>,
}

/// Materialize the visible Control Panel items for the current Windows user.
///
/// The result is built during a scan, so registry/Shell work is kept away from
/// the per-keystroke query path. A failing applet, icon or Shell item is simply
/// skipped; it cannot abort the rest of the application scan.
pub fn materialize_entries() -> Vec<AppItem> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(collect_entries));
    if initialized {
        unsafe {
            CoUninitialize();
        }
    }
    result.unwrap_or_default()
}

fn collect_entries() -> Vec<AppItem> {
    materialize_shell_entries(enumerate_shell_entries())
}

fn materialize_shell_entries(shell_entries: Vec<ShellEntry>) -> Vec<AppItem> {
    let mut items = Vec::with_capacity(shell_entries.len());
    let mut seen_guids = HashSet::new();

    for shell_entry in shell_entries {
        let identity = shell_entry
            .guid
            .as_deref()
            .unwrap_or(&shell_entry.parsing_name)
            .to_ascii_lowercase();
        if !seen_guids.insert(identity.clone()) {
            continue;
        }
        let metadata = read_metadata(shell_entry.guid.as_deref());
        let name = shell_entry
            .name
            .trim()
            .to_string()
            .is_empty()
            .then(|| metadata.fallback_name.clone())
            .flatten()
            .unwrap_or_else(|| shell_entry.name.trim().to_string());
        if name.is_empty() {
            continue;
        }

        let Some(shell_target) = shell_target(&shell_entry.parsing_name) else {
            continue;
        };
        let (target, args) = match (metadata.target, metadata.args) {
            (Some(target), args) => (target, args),
            (None, _) => (shell_target, None),
        };
        let working_dir = Path::new(&target)
            .parent()
            .filter(|path| path.is_dir())
            .map(|path| path.to_string_lossy().to_string());
        let mut keywords = Vec::new();
        for value in [
            metadata.default_name,
            metadata.application_name,
            Some(shell_entry.parsing_name.clone()),
        ]
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && !value.starts_with('@'))
        {
            if !keywords
                .iter()
                .any(|keyword: &String| keyword.eq_ignore_ascii_case(&value))
            {
                keywords.push(value);
            }
        }
        if let Some(module) = args.as_deref().and_then(control_panel_module_name) {
            if !keywords
                .iter()
                .any(|keyword| keyword.eq_ignore_ascii_case(&module))
            {
                keywords.push(module);
            }
        }

        let id = metadata
            .guid
            .as_deref()
            .map(|guid| format!("control-panel:{}", guid.to_ascii_lowercase()))
            .unwrap_or_else(|| format!("control-panel:{}", identity));
        let mut item = AppItem::scanned(
            id,
            name,
            target.clone(),
            args,
            working_dir,
            "builtin-system",
        );
        item.icon_src = metadata.icon_src.or_else(|| Some(target));
        item.search_keywords = keywords;
        item.attach_search_fields();
        items.push(item);
    }
    items
}

fn enumerate_shell_entries() -> Vec<ShellEntry> {
    let folder: IShellItem = unsafe {
        match SHCreateItemFromParsingName(
            PCWSTR(wide(CONTROL_PANEL_FOLDER).as_ptr()),
            None::<&IBindCtx>,
        ) {
            Ok(folder) => folder,
            Err(error) => {
                crate::log::info(&format!("control-panel: open Shell folder failed: {error}"));
                return Vec::new();
            }
        }
    };
    let enum_items: IEnumShellItems = unsafe {
        match folder.BindToHandler(None::<&IBindCtx>, &BHID_EnumItems) {
            Ok(items) => items,
            Err(error) => {
                crate::log::info(&format!("control-panel: bind enum failed: {error}"));
                return Vec::new();
            }
        }
    };

    let mut entries = Vec::new();
    loop {
        let mut fetched = 0u32;
        let mut slot: Option<IShellItem> = None;
        let ok = unsafe {
            enum_items
                .Next(std::slice::from_mut(&mut slot), Some(&mut fetched))
                .is_ok()
        };
        if !ok || fetched == 0 {
            break;
        }
        let Some(item) = slot else { continue };
        let Some(name) = shell_display_name(&item, SIGDN_NORMALDISPLAY) else {
            continue;
        };
        let Some(parsing_name) = shell_display_name(&item, SIGDN_DESKTOPABSOLUTEPARSING) else {
            continue;
        };
        let guid = last_guid(&parsing_name);
        entries.push(ShellEntry {
            name,
            parsing_name,
            guid,
        });
    }
    entries
}

fn shell_display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    let ptr: PWSTR = unsafe { item.GetDisplayName(kind).ok()? };
    if ptr.is_null() {
        return None;
    }
    let name = unsafe { ptr.to_string().ok() };
    unsafe {
        let _ = CoTaskMemFree(Some(ptr.0 as *const _));
    }
    name
}

fn shell_target(parsing_name: &str) -> Option<String> {
    let target = if parsing_name.to_ascii_lowercase().starts_with("shell:") {
        parsing_name.to_string()
    } else {
        format!("shell:{parsing_name}")
    };
    let _resolved: IShellItem = unsafe {
        SHCreateItemFromParsingName(PCWSTR(wide(&target).as_ptr()), None::<&IBindCtx>).ok()?
    };
    Some(target)
}

fn last_guid(value: &str) -> Option<String> {
    let start = value.rfind("::{")? + 2;
    let end = value[start..].find('}')? + start + 1;
    let guid = &value[start..end];
    is_guid_name(guid).then(|| guid.to_string())
}

fn read_metadata(guid: Option<&str>) -> ControlPanelMetadata {
    let Some(guid) = guid else {
        return ControlPanelMetadata::default();
    };
    let clsid_path = format!(r"CLSID\{guid}");
    let default_name = read_registry_string(HKEY_CLASSES_ROOT, &clsid_path, "", None);
    let localized_name =
        read_registry_string(HKEY_CLASSES_ROOT, &clsid_path, "LocalizedString", None);
    let fallback_name = localized_name
        .as_deref()
        .and_then(resolve_indirect_string)
        .or_else(|| localized_name.filter(|value| !value.trim_start().starts_with('@')))
        .or_else(|| default_name.clone())
        .filter(|value| !value.trim().is_empty() && !value.trim_start().starts_with('@'));
    let command_path = format!(r"{clsid_path}\Shell\Open\Command");
    let (target, args) = read_registry_string(HKEY_CLASSES_ROOT, &command_path, "", None)
        .and_then(|command| parse_registry_open_command(&command))
        .map_or((None, None), |(target, args)| (Some(target), args));
    let icon_path = format!(r"{clsid_path}\DefaultIcon");
    let icon_src = read_registry_string(HKEY_CLASSES_ROOT, &icon_path, "", None)
        .map(|value| {
            value
                .trim()
                .trim_matches('"')
                .trim_start_matches('@')
                .to_string()
        })
        .filter(|value| !value.is_empty());
    ControlPanelMetadata {
        guid: Some(guid.to_string()),
        fallback_name,
        default_name,
        application_name: read_registry_string(
            HKEY_CLASSES_ROOT,
            &clsid_path,
            "System.ApplicationName",
            None,
        ),
        target,
        args,
        icon_src,
    }
}

fn parse_registry_open_command(command: &str) -> Option<(String, Option<String>)> {
    let expanded = crate::system::env::expand_env(command.trim());
    let command = expanded.trim();
    if command.is_empty() {
        return None;
    }
    let (raw_target, raw_args) = if let Some(rest) = command.strip_prefix('"') {
        let end = rest.find('"')?;
        (&rest[..end], rest[end + 1..].trim())
    } else {
        let end = command.find(char::is_whitespace).unwrap_or(command.len());
        (&command[..end], command[end..].trim())
    };
    let target = resolve_registry_executable(raw_target)?;
    let args = (!raw_args.is_empty()).then(|| raw_args.to_string());
    Some((target, args))
}

fn resolve_registry_executable(raw_target: &str) -> Option<String> {
    let path = PathBuf::from(raw_target.trim().trim_matches('"'));
    if path.is_file() {
        return Some(path.to_string_lossy().to_string());
    }
    if path.components().count() == 1 {
        let system_root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system_path = system_root.join("System32").join(path);
        if system_path.is_file() {
            return Some(system_path.to_string_lossy().to_string());
        }
    }
    None
}

fn control_panel_module_name(args: &str) -> Option<String> {
    args.split_whitespace()
        .map(|part| part.trim_matches('"').trim_matches(','))
        .find(|part| {
            let lower = part.to_ascii_lowercase();
            lower.ends_with(".cpl") || lower.ends_with(".dll")
        })
        .and_then(|part| {
            Path::new(part)
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
}

fn is_guid_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && [9usize, 14, 19, 24]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 0 && *index != 37 && ![9, 14, 19, 24].contains(index))
            .all(|(_, byte)| byte.is_ascii_hexdigit())
}

fn open_registry_key(hive: HKEY, path: &str, view: Option<u32>) -> Option<HKEY> {
    let wide = path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut key = HKEY::default();
    let result = unsafe { RegOpenKeyExW(hive, PCWSTR(wide.as_ptr()), view, KEY_READ, &mut key) };
    (result == ERROR_SUCCESS).then_some(key)
}

fn read_registry_string(hive: HKEY, path: &str, value: &str, view: Option<u32>) -> Option<String> {
    let key = open_registry_key(hive, path, view)?;
    let name = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut data = vec![0u8; 1024];
    let mut data_len = data.len() as u32;
    let mut ty = REG_SZ;
    let mut result = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut ty),
            Some(data.as_mut_ptr()),
            Some(&mut data_len),
        )
    };
    if result == ERROR_MORE_DATA {
        if data_len == 0 {
            unsafe {
                let _ = RegCloseKey(key);
            }
            return None;
        }
        data.resize(data_len as usize, 0);
        result = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    if result != ERROR_SUCCESS || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
        return None;
    }
    let mut units = data[..data_len as usize]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    while units.last() == Some(&0) {
        units.pop();
    }
    let text = String::from_utf16_lossy(&units);
    if ty == REG_EXPAND_SZ {
        Some(crate::system::env::expand_env(&text))
    } else {
        Some(text)
    }
}

fn resolve_indirect_string(value: &str) -> Option<String> {
    let value = crate::system::env::expand_env(value.trim());
    if !value.starts_with('@') {
        return Some(value);
    }
    let source = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut output = vec![0u16; 1024];
    unsafe {
        SHLoadIndirectString(PCWSTR(source.as_ptr()), &mut output, None).ok()?;
    }
    let end = output
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(output.len());
    let text = String::from_utf16_lossy(&output[..end]).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_shell_namespace_contains_visible_control_panel_items() {
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        let shell_entries = enumerate_shell_entries();
        let visible: HashSet<String> = shell_entries
            .iter()
            .filter(|entry| !entry.name.trim().is_empty())
            .filter(|entry| shell_target(&entry.parsing_name).is_some())
            .map(|entry| {
                let identity = entry.guid.as_deref().unwrap_or(&entry.parsing_name);
                format!("control-panel:{}", identity.to_ascii_lowercase())
            })
            .collect();
        let entries = materialize_shell_entries(shell_entries);
        let indexed: HashSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
        assert!(!visible.is_empty(), "控制面板 Shell 命名空间应可枚举");
        let missing: Vec<_> = visible.difference(&indexed).collect();
        assert!(
            missing.is_empty(),
            "有名称且可解析的可见项漏索引: {missing:?}"
        );
        if initialized {
            unsafe { CoUninitialize() };
        }
    }

    #[test]
    fn shell_entries_have_verified_targets() {
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        let entries = materialize_entries();
        assert!(!entries.is_empty());
        for entry in entries {
            if entry.target.starts_with("shell:") {
                let _: IShellItem = unsafe {
                    SHCreateItemFromParsingName(
                        PCWSTR(wide(&entry.target).as_ptr()),
                        None::<&IBindCtx>,
                    )
                }
                .expect("Shell target must remain resolvable");
            } else {
                assert!(
                    Path::new(&entry.target).is_file(),
                    "Control Panel target must exist: {}",
                    entry.target
                );
            }
        }
        if initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}
