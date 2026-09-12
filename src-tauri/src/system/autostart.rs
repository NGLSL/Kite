//! 开机启动：写注册表 Run 键。可用 Settings 开关。

use std::os::windows::ffi::OsStrExt;

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
    REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "Kite";

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 注册/取消开机启动。`enable` 为 true 时写入带引号的 exe 路径。
pub fn set_autostart(enable: bool) -> Result<(), String> {
    let key_wide = wide(RUN_KEY);
    let mut hkey = Default::default();
    unsafe {
        let open = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            Some(0),
            KEY_SET_VALUE,
            &mut hkey,
        );
        if open.is_err() {
            return Err(format!("open Run key failed: {open:?}"));
        }

        if enable {
            let path = format!("\"{}\"", exe_path());
            let value: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let name = wide(VALUE_NAME);
            let set = RegSetValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    value.as_ptr() as *const u8,
                    value.len() * 2,
                )),
            );
            let _ = RegCloseKey(hkey);
            if set.is_err() {
                return Err(format!("set autostart failed: {set:?}"));
            }
        } else {
            let name = wide(VALUE_NAME);
            let _ = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
            let _ = RegCloseKey(hkey);
        }
    }
    Ok(())
}
