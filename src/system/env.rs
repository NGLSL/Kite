//! 子进程环境刷新：启动器常驻后台，进程环境停在登录时刻。
//! 期间在「系统/用户环境变量」里新增或修改的条目（典型：给机器级 PATH 追加新工具），
//! 子进程拿不到——表现为 VS Code/IDEA 内置终端里 PATH 不全（uTools/Flow 同类问题；
//! Windows 搜索正常是因为 Explorer 监听了环境变更广播）。
//!
//! 每次启动前从注册表 HKLM+HKCU 的 Environment 重读并写入本进程：
//! - 只增改不删：会话变量（SESSIONNAME 等）与 dev 会话里临时加的 PATH 段不受影响
//! - PATH 特殊合并：当前段在前，按登录规则补注册表缺失的机器段、用户段
//! 之后 Command / ShellExecute 拉起的子进程随之继承新环境。

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_EXPAND_SZ, REG_SZ,
};

const MACHINE_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const USER_ENV: &str = "Environment";

/// 扫描命令入口时使用当前会话与注册表中的 PATH，覆盖启动器常驻期间新装的 CLI。
pub fn effective_path() -> String {
    let current = std::env::var_os("PATH")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let machine = read_hive(HKEY_LOCAL_MACHINE, MACHINE_ENV);
    let user = read_hive(HKEY_CURRENT_USER, USER_ENV);
    merge_path(
        &current,
        machine.get("PATH").map(String::as_str),
        user.get("PATH").map(String::as_str),
    )
}

/// 拉起子进程前调用。失败静默（读不到注册表就不改环境）。
pub fn refresh_process_env() {
    let current: HashMap<String, String> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .map(|(k, v)| (k.to_uppercase(), v))
        .collect();
    let machine = read_hive(HKEY_LOCAL_MACHINE, MACHINE_ENV);
    let user = read_hive(HKEY_CURRENT_USER, USER_ENV);
    for (k, v) in planned_updates(&current, &machine, &user) {
        std::env::set_var(&k, &v);
    }
}

/// 计算需要写入本进程的 (名, 值)。纯函数便于测试；键均已大写。
fn planned_updates(
    current: &HashMap<String, String>,
    machine: &HashMap<String, String>,
    user: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    // PATH：当前段在前，补注册表缺失段
    let cur = current.get("PATH").cloned().unwrap_or_default();
    let merged = merge_path(
        &cur,
        machine.get("PATH").map(String::as_str),
        user.get("PATH").map(String::as_str),
    );
    if merged != cur {
        out.push(("Path".into(), merged));
    }

    // 其余变量：注册表与当前不同才写；同键先机器后用户，后写即用户优先
    for (k, v) in machine.iter().chain(user.iter()) {
        if k == "PATH" {
            continue;
        }
        if current.get(k).map(String::as_str) != Some(v.as_str()) {
            out.push((k.clone(), v.clone()));
        }
    }
    out
}

/// 当前段在前；补注册表缺失段（机器在前、用户在后），大小写不敏感去重，丢空段。
fn merge_path(current: &str, machine: Option<&str>, user: Option<&str>) -> String {
    let mut segs: Vec<String> = current
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let mut seen: HashSet<String> = segs.iter().map(|s| s.to_lowercase()).collect();
    for part in [machine, user].into_iter().flatten() {
        for seg in part.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            if seen.insert(seg.to_lowercase()) {
                segs.push(seg.to_string());
            }
        }
    }
    segs.join(";")
}

/// 读取注册表环境块（仅字符串值；REG_EXPAND_SZ 现场展开）。失败返回空表。
fn read_hive(hive: HKEY, subkey: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let wide: Vec<u16> = OsStr::new(subkey)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(hive, PCWSTR(wide.as_ptr()), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return out;
        }
        let mut index = 0u32;
        loop {
            let mut name_buf = [0u16; 512];
            let mut name_len = name_buf.len() as u32;
            let mut ty = 0u32;
            let mut data = vec![0u8; 2048];
            let mut data_len = data.len() as u32;
            let err = RegEnumValueW(
                key,
                index,
                Some(PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut ty),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            );
            if err == ERROR_MORE_DATA {
                // 值超过缓冲（典型：很长的 PATH）：按所需大小对同一条目重读一次
                data.resize(data_len as usize, 0);
                name_len = name_buf.len() as u32;
                if RegEnumValueW(
                    key,
                    index,
                    Some(PWSTR(name_buf.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut ty),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                ) != ERROR_SUCCESS
                {
                    break;
                }
            } else if err != ERROR_SUCCESS {
                break; // ERROR_NO_MORE_ITEMS 或异常
            }
            if name_len > 0 {
                if let Some(v) = value_to_string(ty, &data[..data_len as usize]) {
                    let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
                    out.insert(name.to_uppercase(), v);
                }
            }
            index += 1;
        }
        let _ = RegCloseKey(key);
    }
    out
}

/// REG_SZ / REG_EXPAND_SZ → 字符串；展开 %VAR% 用当前进程环境。
fn value_to_string(ty: u32, data: &[u8]) -> Option<String> {
    if ty != REG_SZ.0 && ty != REG_EXPAND_SZ.0 {
        return None;
    }
    let u16s: Vec<u16> = data
        .chunks_exact(2)
        .take_while(|c| c[0] != 0)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let s = String::from_utf16_lossy(&u16s);
    if ty == REG_EXPAND_SZ.0 {
        Some(expand_env(&s))
    } else {
        Some(s)
    }
}

/// 展开 `%windir%` 等环境变量；失败原样返回。
pub(crate) fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let wide: Vec<u16> = OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
        let need = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None);
        if need == 0 {
            return s.to_string();
        }
        let mut buf = vec![0u16; need as usize];
        let n = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf));
        if n == 0 || n > need {
            return s.to_string();
        }
        String::from_utf16_lossy(&buf[..n as usize - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_uppercase(), v.to_string()))
            .collect()
    }

    #[test]
    fn merge_path_appends_missing_machine_then_user() {
        let merged = merge_path(
            r"C:\a;C:\b",
            Some(r"C:\Windows\System32;C:\a"),
            Some(r"C:\Users\x\bin;c:\A"),
        );
        assert_eq!(
            merged, r"C:\a;C:\b;C:\Windows\System32;C:\Users\x\bin",
            "缺段按 机器→用户 追加,大小写不敏感去重"
        );
    }

    #[test]
    fn merge_path_handles_missing_registry_parts() {
        assert_eq!(merge_path("C:\\a", None, Some("C:\\b")), "C:\\a;C:\\b");
        assert_eq!(merge_path("C:\\a", Some("C:\\a"), None), "C:\\a");
        assert_eq!(merge_path("", Some("C:\\a"), None), "C:\\a");
    }

    #[test]
    fn planned_updates_only_changed() {
        let current = map(&[("PATH", "C:\\a"), ("JAVA_HOME", "C:\\old")]);
        let machine = map(&[("PATH", "C:\\Windows;C:\\a"), ("JAVA_HOME", "C:\\old")]);
        let user = map(&[("PATH", "C:\\user"), ("JAVA_HOME", "C:\\new")]);
        let updates = planned_updates(&current, &machine, &user);
        // JAVA_HOME: 当前与机器一致、与用户不同 → 写用户值(优先)
        assert!(
            updates.contains(&("JAVA_HOME".into(), "C:\\new".into())),
            "updates={updates:?}"
        );
        // PATH: 补机器与用户缺段
        let path = updates.iter().find(|(k, _)| k == "Path").unwrap();
        assert_eq!(path.1, r"C:\a;C:\Windows;C:\user");
        // 未变化的变量不出现
        assert!(!updates.iter().any(|(k, _)| k == "TEMP"));
    }

    #[test]
    fn value_parsing() {
        assert_eq!(
            value_to_string(REG_SZ.0, &[0x61, 0x00, 0x62, 0x00, 0, 0]),
            Some("ab".into())
        );
        let expanded = value_to_string(REG_EXPAND_SZ.0, &{
            let mut v: Vec<u8> = "%SystemRoot%\\x"
                .encode_utf16()
                .flat_map(|c| c.to_le_bytes())
                .collect();
            v.extend_from_slice(&[0, 0]);
            v
        })
        .unwrap();
        assert!(expanded.ends_with("\\x"), "{expanded}");
        assert!(!expanded.contains('%'), "{expanded}");
        // DWORD 等非文本类型跳过
        assert_eq!(value_to_string(4, &[1, 0, 0, 0]), None);
    }

    /// 真机冒烟：读注册表并应用，PATH 应保持非空且进程仍能解析系统目录。
    #[test]
    fn refresh_smoke_on_real_registry() {
        let before = std::env::var("PATH").unwrap_or_default();
        refresh_process_env();
        let after = std::env::var("PATH").unwrap_or_default();
        assert!(!after.is_empty());
        // 只增改不删：原段都还在（忽略大小写）
        for seg in before.split(';').filter(|s| !s.is_empty()) {
            assert!(
                after.split(';').any(|s| s.eq_ignore_ascii_case(seg)),
                "原 PATH 段被删: {seg}"
            );
        }
    }
}
