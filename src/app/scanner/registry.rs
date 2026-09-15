//! 从注册表 App Paths 读取已注册应用。
//! 64/32 位视图都读（32 位安装的 Office 只注册在 WOW6432Node）；
//! 跳过指向不存在文件的死条目（卸载残留会带着假路径留在 64 位视图里）。

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, REG_SZ,
};

use super::util::wide;
use crate::model::AppItem;

type RawItem = (AppItem, Option<String>);

pub fn collect_app_paths(source: &str, out: &mut Vec<RawItem>) {
    let subkey = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    for (hive, options) in [
        (HKEY_LOCAL_MACHINE, None),
        (HKEY_LOCAL_MACHINE, Some(wow64_32())),
        (HKEY_CURRENT_USER, None),
        (HKEY_CURRENT_USER, Some(wow64_32())),
    ] {
        collect_hive(hive, subkey, options, source, out);
    }
}

/// 以 32 位视图打开（RegOpenKeyExW 的 uloptions 参数）
fn wow64_32() -> u32 {
    KEY_WOW64_32KEY.0
}

fn collect_hive(
    hive: HKEY,
    subkey: &str,
    options: Option<u32>,
    source: &str,
    out: &mut Vec<RawItem>,
) {
    let subkey_wide = wide(subkey);
    let mut hkey = Default::default();
    unsafe {
        if RegOpenKeyExW(
            hive,
            PCWSTR(subkey_wide.as_ptr()),
            options,
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return;
        }
    }

    let mut index = 0u32;
    loop {
        let Some(sub) = enum_subkey(hkey, index) else {
            break;
        };
        if let Some(target) = read_default_path(hive, &format!("{subkey}\\{sub}"), options) {
            if let Some(item) = app_path_item(source, &sub, &target) {
                out.push(item);
            }
        }
        index += 1;
    }

    unsafe {
        let _ = RegCloseKey(hkey);
    }
}

fn app_path_item(source: &str, sub: &str, raw_target: &str) -> Option<RawItem> {
    // App Paths 默认值是可执行文件路径；部分安装器会用引号包裹路径。
    // 引号属于注册表值的表示方式，不是文件名的一部分。
    let raw_target = raw_target.trim();
    let path = raw_target
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw_target)
        .trim();
    // 展开 %ProgramFiles% 等，并过滤卸载残留的死条目。
    let target = crate::system::env::expand_env(path);
    if !Path::new(&target).exists() {
        crate::log::info(&format!("app-paths: skip missing {sub} -> {target}"));
        return None;
    }
    let name = Path::new(&target)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| sub.trim_end_matches(".exe").to_string());
    let working_dir = Path::new(&target)
        .parent()
        .map(|p| p.to_string_lossy().to_string());
    let icon_src = Some(target.clone());
    let item = AppItem::scanned(
        super::util::stable_item_id(&target, None),
        name,
        target,
        None,
        working_dir,
        source,
    );
    Some((item, icon_src))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_app_path_remains_searchable() {
        let dir = std::env::temp_dir().join(format!("kite-quoted-app-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ExamplePlayer.exe");
        std::fs::write(&exe, b"fixture").unwrap();

        let registered = format!("  \"{}\"  ", exe.display());
        let (mut item, _) = app_path_item("app-paths", "ExamplePlayer.exe", &registered)
            .expect("a quoted existing App Paths executable must be indexed");
        item.attach_search_fields();
        let index = crate::search::RetrievalIndex::build(&[item], &[]);
        assert!(
            index
                .search("ExamplePlayer", &[], 10)
                .iter()
                .any(|hit| hit.item.name == "ExamplePlayer"),
            "the registered application must be searchable by its executable name"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

fn enum_subkey(hkey: HKEY, index: u32) -> Option<String> {
    let mut name_buf = [0u16; 256];
    let mut name_len = name_buf.len() as u32;
    let ok = unsafe {
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
    if !ok {
        return None;
    }
    Some(String::from_utf16_lossy(&name_buf[..name_len as usize]))
}

fn read_default_path(hive: HKEY, subkey: &str, options: Option<u32>) -> Option<String> {
    let subkey_wide = wide(subkey);
    let mut child = Default::default();
    unsafe {
        if RegOpenKeyExW(
            hive,
            PCWSTR(subkey_wide.as_ptr()),
            options,
            KEY_READ,
            &mut child,
        )
        .is_err()
        {
            return None;
        }
    }

    let mut data = vec![0u8; 1024];
    let mut data_len = data.len() as u32;
    let mut ty = REG_SZ;
    let empty = wide("");
    let ok = unsafe {
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
    unsafe {
        let _ = RegCloseKey(child);
    }
    if !ok {
        return None;
    }

    let target = u16_slice_to_string(&data, data_len);
    let target = target.trim_matches('\0').trim().to_string();
    if target.is_empty() {
        None
    } else {
        Some(target)
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
