//! 从注册表 App Paths 读取已注册应用。

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
};

use super::util::{hash_id, normalize_path_key, wide};
use crate::model::AppItem;

type RawItem = (AppItem, Option<String>);

pub fn collect_app_paths(source: &str, out: &mut Vec<RawItem>) {
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
        collect_hive(hive, subkey, source, out);
    }
}

fn collect_hive(hive: HKEY, subkey: &str, source: &str, out: &mut Vec<RawItem>) {
    let subkey_wide = wide(subkey);
    let mut hkey = Default::default();
    unsafe {
        if RegOpenKeyExW(hive, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut hkey).is_err() {
            return;
        }
    }

    let mut index = 0u32;
    loop {
        let Some(sub) = enum_subkey(hkey, index) else {
            break;
        };
        if let Some(target) = read_default_path(hive, &format!("{subkey}\\{sub}")) {
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
        index += 1;
    }

    unsafe {
        let _ = RegCloseKey(hkey);
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

fn read_default_path(hive: HKEY, subkey: &str) -> Option<String> {
    let subkey_wide = wide(subkey);
    let mut child = Default::default();
    unsafe {
        if RegOpenKeyExW(hive, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut child).is_err()
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
