//! Windows 系统主题探测（注册表 `AppsUseLightTheme`）。
//! 仅在 Windows 平台通过系统注册表轻量读取，无多余外部依赖。

/// 读取 Windows 注册表判断当前系统是否为深色模式（AppsUseLightTheme == 0）。
/// 若键不存在或非 Windows 平台，安全回退到深色模式（true）。
pub fn is_windows_dark_mode() -> bool {
    #[cfg(windows)]
    {
        use windows::core::w;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
        };

        let subkey = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let value_name = w!("AppsUseLightTheme");
        let mut hkey = windows::Win32::System::Registry::HKEY::default();

        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, None, KEY_READ, &mut hkey).is_ok() {
                let mut data = 0u32;
                let mut data_size = std::mem::size_of::<u32>() as u32;
                let mut r#type = REG_DWORD;
                let status = RegQueryValueExW(
                    hkey,
                    value_name,
                    None,
                    Some(&mut r#type),
                    Some(&mut data as *mut u32 as *mut u8),
                    Some(&mut data_size),
                );
                let _ = RegCloseKey(hkey);
                if status.is_ok() && r#type == REG_DWORD {
                    return data == 0;
                }
            }
        }
    }
    // 默认回退深色
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_windows_dark_mode_does_not_panic() {
        // 在任何 Windows 宿主环境或 CI 下均应安全返回 bool，不触发 panic
        let is_dark = is_windows_dark_mode();
        let _ = is_dark;
    }
}
