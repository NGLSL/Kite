//! UWP / Store 应用：从 Packages 仓库枚举显示名，用 shell:AppsFolder 启动。

use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY_CURRENT_USER, KEY_READ,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::model::AppItem;

use super::scanner::util::{hash_id, normalize_path_key};

/// 补充 Store 应用到扫描结果（失败静默）。
pub fn collect_uwp(source: &str, out: &mut Vec<(AppItem, Option<String>)>) {
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let key = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let key_wide = wide(key);
    let mut hkey = Default::default();
    unsafe {
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return;
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut index = 0u32;
    loop {
        let mut name_buf = [0u16; 512];
        let mut name_len = name_buf.len() as u32;
        let ok = unsafe {
            RegEnumKeyExW(
                hkey,
                index,
                Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            )
            .is_ok()
        };
        if !ok {
            break;
        }
        let full = String::from_utf16_lossy(&name_buf[..name_len as usize]);
        let display = full
            .split('_')
            .next()
            .unwrap_or(&full)
            .replace('.', " ")
            .trim()
            .to_string();
        if display.is_empty() || !seen.insert(display.to_lowercase()) {
            index += 1;
            continue;
        }
        let family = {
            let parts: Vec<&str> = full.split('_').collect();
            if parts.len() >= 5 {
                format!("{}_{}", parts[0], parts[4])
            } else {
                full.clone()
            }
        };
        let target = format!("shell:AppsFolder\\{family}");
        let id = hash_id(&[&normalize_path_key(&target), source]);
        let item = AppItem::scanned(id, display, target, None, None, source);
        out.push((item, None));
        index += 1;
    }
    unsafe {
        let _ = RegCloseKey(hkey);
    }
}

/// 启动 shell:AppsFolder / 普通路径。
pub fn launch_shell_path(target: &str) -> Result<(), String> {
    let file: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let code = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute 返回 HINSTANCE，> 32 为成功
    if code.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed for {target}"))
    }
}
