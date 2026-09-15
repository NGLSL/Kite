//! 从 Windows Uninstall 注册表读取有明确启动目标的已安装桌面应用。
//!
//! 该来源只接受同时具备非空 `DisplayName`、指向现有 `.exe` 的
//! `DisplayIcon` 的条目。`UninstallString` / `QuietUninstallString` 用来
//! 排除把卸载器本身误当成应用的情况；解析失败只影响当前条目。

use std::collections::HashSet;
use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ,
};

use super::util::{normalize_path_key, stable_item_id, wide};
use crate::model::AppItem;

/// 与 scanner 其他来源相同的原始条目形态；图标源为已验证的 exe 路径。
pub type RawItem = (AppItem, Option<String>);

const UNINSTALL_SUBKEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const MAX_REG_VALUE_BYTES: usize = 1024 * 1024;

/// 枚举 HKCU/HKLM 的 64 位和 32 位视图。
///
/// 返回值可直接 `extend` 到 scanner 的原始条目集合；同一目标在不同
/// hive/view 中只保留首次遇到的条目，后续由 scanner 的全局去重继续合并。
pub fn collect_uninstall(source: &str) -> Vec<RawItem> {
    let mut out = Vec::new();
    let mut seen_targets = HashSet::new();

    for (hive, hive_name) in [(HKEY_CURRENT_USER, "HKCU"), (HKEY_LOCAL_MACHINE, "HKLM")] {
        for (view, view_name) in [(KEY_WOW64_64KEY, "64"), (KEY_WOW64_32KEY, "32")] {
            collect_view(
                hive,
                hive_name,
                view,
                view_name,
                source,
                &mut seen_targets,
                &mut out,
            );
        }
    }

    out
}

fn collect_view(
    hive: HKEY,
    hive_name: &str,
    view: windows::Win32::System::Registry::REG_SAM_FLAGS,
    view_name: &str,
    source: &str,
    seen_targets: &mut HashSet<String>,
    out: &mut Vec<RawItem>,
) {
    let root_name = wide(UNINSTALL_SUBKEY);
    let mut root = HKEY::default();
    // KEY_WOW64_* belongs to samDesired, not RegOpenKeyExW's reserved
    // ulOptions parameter. Keeping the view flag here is important for 32-bit
    // software installed beside a 64-bit Kite process.
    let status = unsafe {
        RegOpenKeyExW(
            hive,
            PCWSTR(root_name.as_ptr()),
            None,
            KEY_READ | view,
            &mut root,
        )
    };
    if status != ERROR_SUCCESS {
        return;
    }

    let before = out.len();
    let mut index = 0u32;
    while let Some(subkey) = enum_subkey(root, index) {
        if let Some(entry) = read_entry(root, &subkey, view) {
            if let Some((item, icon_src)) = entry_to_item(source, &entry) {
                let key = target_key(&item.target);
                if seen_targets.insert(key) {
                    out.push((item, icon_src));
                }
            }
        }
        index += 1;
    }

    unsafe {
        let _ = RegCloseKey(root);
    }
    crate::log::info(&format!(
        "uninstall: scanned {hive_name} {view_name}-bit view (+{} entries)",
        out.len() - before
    ));
}

struct UninstallEntry {
    display_name: Option<String>,
    display_icon: Option<String>,
    uninstall: [Option<String>; 2],
}

fn read_entry(
    root: HKEY,
    subkey: &str,
    view: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Option<UninstallEntry> {
    let subkey_name = wide(subkey);
    let mut child = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(subkey_name.as_ptr()),
            None,
            KEY_READ | view,
            &mut child,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }

    let entry = UninstallEntry {
        display_name: read_string_value(child, "DisplayName"),
        display_icon: read_string_value(child, "DisplayIcon"),
        uninstall: [
            read_string_value(child, "UninstallString"),
            read_string_value(child, "QuietUninstallString"),
        ],
    };
    unsafe {
        let _ = RegCloseKey(child);
    }
    Some(entry)
}

fn entry_to_item(source: &str, entry: &UninstallEntry) -> Option<RawItem> {
    let name = clean_display_name(entry.display_name.as_deref()?)?;
    let target = resolve_existing_executable(entry.display_icon.as_deref()?)?;
    let target_path = Path::new(&target);

    // Both the display name and the executable stem are useful signals. Some
    // vendors give their updater a neutral executable name while advertising
    // "Product Updater" in DisplayName; indexing that row would expose a
    // maintenance helper as the primary application.
    if is_auxiliary_target(target_path) || contains_auxiliary_marker(&name) {
        return None;
    }

    for uninstall in entry.uninstall.iter().flatten() {
        let Some(command_target) = clean_command_target(uninstall) else {
            continue;
        };
        let command_target = crate::system::env::expand_env(&command_target);
        if same_target(&target, &command_target) {
            return None;
        }
    }

    let working_dir = target_path
        .parent()
        .map(|path| path.to_string_lossy().to_string());
    let item = AppItem::scanned(
        stable_item_id(&target, None),
        name,
        target.clone(),
        None,
        working_dir,
        source,
    );
    Some((item, Some(target)))
}

fn read_string_value(key: HKEY, value_name: &str) -> Option<String> {
    let name = wide(value_name);
    let mut ty = REG_SZ;
    let mut byte_len = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut ty),
            None,
            Some(&mut byte_len),
        )
    };
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        return None;
    }
    if ty != REG_SZ && ty != REG_EXPAND_SZ {
        return None;
    }
    let byte_len = byte_len as usize;
    if byte_len == 0 || byte_len > MAX_REG_VALUE_BYTES {
        return None;
    }

    let mut data = vec![0u8; byte_len];
    let mut actual_len = byte_len as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut ty),
            Some(data.as_mut_ptr()),
            Some(&mut actual_len),
        )
    };
    if status != ERROR_SUCCESS || actual_len as usize > data.len() {
        return None;
    }

    let value = decode_registry_string(ty, &data[..actual_len as usize])?;
    if ty == REG_EXPAND_SZ {
        Some(crate::system::env::expand_env(&value))
    } else {
        Some(value)
    }
}

fn decode_registry_string(
    ty: windows::Win32::System::Registry::REG_VALUE_TYPE,
    data: &[u8],
) -> Option<String> {
    if ty != REG_SZ && ty != REG_EXPAND_SZ {
        return None;
    }
    if data.len() % 2 != 0 {
        return None;
    }
    let units = data
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let value = String::from_utf16_lossy(&units);
    let value = value.trim_matches('\0').trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn enum_subkey(root: HKEY, index: u32) -> Option<String> {
    // Registry key names are limited to 255 UTF-16 code units.
    let mut name = [0u16; 256];
    let mut length = name.len() as u32;
    let status = unsafe {
        RegEnumKeyExW(
            root,
            index,
            Some(PWSTR(name.as_mut_ptr())),
            &mut length,
            None,
            None,
            None,
            None,
        )
    };
    if status != ERROR_SUCCESS || length == 0 || length as usize > name.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&name[..length as usize]))
}

/// Trim registry string terminators and require a non-empty display name.
fn clean_display_name(raw: &str) -> Option<String> {
    let name = raw.trim_matches('\0').trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Remove registry display-icon quoting and a trailing resource index.
///
/// Examples accepted by Windows installers include `"C:\\app\\app.exe",0`,
/// `C:\\app\\app.exe,-101`, and an environment-variable-prefixed equivalent.
fn clean_display_icon(raw: &str) -> Option<String> {
    let value = raw.trim_matches('\0').trim();
    if value.is_empty() {
        return None;
    }

    let path = if let Some(quoted) = value.strip_prefix('"') {
        let end = quoted.find('"')?;
        &quoted[..end]
    } else {
        value
    };
    let path = strip_resource_index(path).trim().trim_matches('"').trim();
    (!path.is_empty()).then(|| path.to_string())
}

fn strip_resource_index(value: &str) -> &str {
    let Some((path, suffix)) = value.rsplit_once(',') else {
        return value;
    };
    let suffix = suffix.trim();
    let digits = suffix.strip_prefix('-').unwrap_or(suffix);
    if !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()) {
        path.trim_end()
    } else {
        value
    }
}

fn resolve_existing_executable(raw: &str) -> Option<String> {
    let path = clean_display_icon(raw)?;
    let path = crate::system::env::expand_env(&path);
    let path = path.trim();
    let candidate = Path::new(path);
    if !candidate.is_file()
        || !candidate
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    Some(path.to_string())
}

/// Extract only the executable portion from an uninstall command line.
fn clean_command_target(raw: &str) -> Option<String> {
    let value = raw.trim_matches('\0').trim();
    if value.is_empty() {
        return None;
    }

    let target = if let Some(quoted) = value.strip_prefix('"') {
        let end = quoted.find('"')?;
        &quoted[..end]
    } else if let Some(end) = value.to_ascii_lowercase().find(".exe") {
        &value[..end + ".exe".len()]
    } else {
        value.split_whitespace().next()?
    };
    let target = target.trim().trim_matches('"').trim();
    (!target.is_empty()).then(|| target.to_string())
}

fn is_auxiliary_target(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .or_else(|| path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    contains_auxiliary_marker(&stem)
}

fn contains_auxiliary_marker(value: &str) -> bool {
    const MARKERS: &[&str] = &[
        "uninstall",
        "unins",
        "setup",
        "install",
        "updater",
        "update",
        "crashpad",
        "crashreport",
        "crash-reporter",
        "helper",
        "repair",
        "maintenance",
        "runtime",
        "redistributable",
        "webview",
    ];
    let value = value.to_ascii_lowercase();
    MARKERS.iter().any(|marker| value.contains(marker))
}

fn target_key(path: &str) -> String {
    let mut normalized = normalize_path_key(path.trim().trim_matches('"'));
    if let Some(without_prefix) = normalized.strip_prefix(r"\\?\") {
        normalized = without_prefix.to_string();
    }
    normalized
}

fn same_target(left: &str, right: &str) -> bool {
    target_key(left) == target_key(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_icon_parser_removes_quotes_and_resource_index() {
        assert_eq!(
            clean_display_icon(r#"  "%ProgramFiles%\Example\Example.exe",0  "#),
            Some(r#"%ProgramFiles%\Example\Example.exe"#.into())
        );
        assert_eq!(
            clean_display_icon(r#"C:\Example\Example.exe,-7"#),
            Some(r#"C:\Example\Example.exe"#.into())
        );
    }

    #[test]
    fn display_icon_parser_keeps_commas_that_are_part_of_the_path() {
        assert_eq!(
            clean_display_icon(r#"C:\Vendor, Inc\Example.exe"#),
            Some(r#"C:\Vendor, Inc\Example.exe"#.into())
        );
    }

    #[test]
    fn uninstall_command_parser_extracts_the_executable_only() {
        assert_eq!(
            clean_command_target(r#""C:\Program Files\Example\uninstall.exe" /S"#),
            Some(r#"C:\Program Files\Example\uninstall.exe"#.into())
        );
        assert_eq!(
            clean_command_target(r#"C:\Program Files\Example\uninstall.exe /quiet"#),
            Some(r#"C:\Program Files\Example\uninstall.exe"#.into())
        );
    }

    #[test]
    fn auxiliary_targets_are_rejected_case_insensitively() {
        for target in [
            r#"C:\Example\unins000.exe"#,
            r#"C:\Example\Setup.exe"#,
            r#"C:\Example\ExampleUpdater.exe"#,
            r#"C:\Example\CrashpadHandler.exe"#,
            r#"C:\Example\plugin-helper.exe"#,
        ] {
            assert!(is_auxiliary_target(Path::new(target)), "{target}");
        }
        assert!(!is_auxiliary_target(Path::new(r#"C:\Example\Example.exe"#)));
    }

    #[test]
    fn display_name_must_be_present() {
        assert_eq!(clean_display_name("  Example  \0"), Some("Example".into()));
        assert_eq!(clean_display_name("   \0"), None);
    }

    #[test]
    fn registry_strings_require_utf16_text_and_trim_terminators() {
        let data: Vec<u8> = "Example\0"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(
            decode_registry_string(REG_SZ, &data),
            Some("Example".into())
        );
        assert_eq!(decode_registry_string(REG_SZ, &[0x45]), None);
        assert_eq!(
            decode_registry_string(windows::Win32::System::Registry::REG_DWORD, &data,),
            None
        );
    }

    #[test]
    fn target_comparison_normalizes_case_slashes_and_extended_prefix() {
        assert!(same_target(
            r#"C:/Program Files/Example/Example.exe"#,
            r#"\\?\C:\Program Files\Example\Example.exe"#,
        ));
    }

    #[test]
    fn entry_requires_a_real_exe_and_rejects_the_uninstall_target() {
        let root =
            std::env::temp_dir().join(format!("kite-uninstall-entry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let app = root.join("Example.exe");
        let other = root.join("Other.exe");
        std::fs::write(&app, b"fixture").unwrap();
        std::fs::write(&other, b"fixture").unwrap();

        let rejected = UninstallEntry {
            display_name: Some("Example".into()),
            display_icon: Some(format!(r#""{}",0"#, app.display())),
            uninstall: [Some(format!(r#""{}" /S"#, app.display())), None],
        };
        assert!(entry_to_item("uninstall", &rejected).is_none());

        let accepted = UninstallEntry {
            display_name: Some("Example".into()),
            display_icon: Some(format!(r#""{}",0"#, app.display())),
            uninstall: [Some(format!(r#""{}" /S"#, other.display())), None],
        };
        let (item, icon) = entry_to_item("uninstall", &accepted).unwrap();
        assert_eq!(item.name, "Example");
        assert_eq!(item.target, app.to_string_lossy());
        assert_eq!(icon.as_deref(), Some(app.to_string_lossy().as_ref()));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn display_icon_expands_environment_variables_before_validation() {
        let root = std::env::temp_dir().join(format!("kite-uninstall-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let exe = root.join("Example.exe");
        std::fs::write(&exe, b"fixture").unwrap();

        let variable = format!("KITE_UNINSTALL_FIXTURE_{}", std::process::id());
        std::env::set_var(&variable, &root);
        let raw = format!(r#"%{variable}%\Example.exe,0"#);
        let resolved = resolve_existing_executable(&raw);
        assert_eq!(resolved.as_deref(), Some(exe.to_string_lossy().as_ref()));
        std::env::remove_var(&variable);
        let _ = std::fs::remove_dir_all(root);
    }
}
