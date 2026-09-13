//! 打包应用与文件路径的激活：ApplicationActivationManager 与 ShellExecute。
//! 从 uwp/mod.rs 拆出，专注"怎么启动"；枚举逻辑见 mod.rs。

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, IApplicationActivationManager, AO_NONE, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::super::scanner::util::wide;

/// 启动 shell:AppsFolder / 普通路径 / UWP AUMID。
pub fn launch_shell_path(target: &str) -> Result<(), String> {
    if let Some(aumid) = target.strip_prefix("shell:AppsFolder\\") {
        if !aumid.is_empty() {
            return activate_uwp(aumid);
        }
    }

    let file: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    // 给处理进程明确的 cwd（文件所在目录 / 主目录），避免继承 Kite 安装目录
    let dir = shell_working_dir(target);
    let dir_pc = dir
        .as_ref()
        .map(|d| PCWSTR(d.as_ptr()))
        .unwrap_or(PCWSTR::null());
    let code = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            dir_pc,
            SW_SHOWNORMAL,
        )
    };
    if code.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed for {target}"))
    }
}

/// ShellExecute 的 lpDirectory：文件夹/文件用其所在目录，其余（URI）用用户主目录。
fn shell_working_dir(target: &str) -> Option<Vec<u16>> {
    let p = Path::new(target);
    let dir = if p.is_dir() {
        Some(p.to_path_buf())
    } else {
        p.parent().filter(|d| d.is_dir()).map(|d| d.to_path_buf())
    };
    let dir = dir.or_else(dirs::home_dir)?;
    Some(wide(&dir.to_string_lossy()))
}

/// 以管理员运行（ShellExecute "runas"，触发 UAC 授权框）。
/// 供 launcher 在 CreateProcess 报 os error 740（需要提升）时兜底，
/// 与资源管理器双击 requireAdministrator 程序的行为一致。
pub fn launch_runas(target: &str, args: &str, working_dir: Option<&Path>) -> Result<(), String> {
    let verb = wide("runas");
    let file = wide(target);
    let params = if args.is_empty() { None } else { Some(wide(args)) };
    let dir = working_dir.map(|d| wide(&d.to_string_lossy()));
    let code = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            params
                .as_ref()
                .map(|p| PCWSTR(p.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            dir.as_ref()
                .map(|d| PCWSTR(d.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            SW_SHOWNORMAL,
        )
    };
    if code.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "runas launch failed for {target} (ShellExecute code {})",
            code.0 as isize
        ))
    }
}

/// 打包应用激活不接收 cwd：激活出的进程要么继承激活方 cwd（Kite 安装目录），
/// 要么落到包安装目录；终端类应用（startingDirectory=null）会原样展示给用户。
/// 激活前把进程 cwd 切到用户主目录，结束后恢复。
fn activate_uwp(aumid: &str) -> Result<(), String> {
    let prev = std::env::current_dir().ok();
    if let Some(home) = dirs::home_dir() {
        let _ = std::env::set_current_dir(home);
    }
    let result = activate_uwp_inner(aumid);
    if let Some(p) = prev {
        let _ = std::env::set_current_dir(p);
    }
    result
}

fn activate_uwp_inner(aumid: &str) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let aam: IApplicationActivationManager =
            CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("ActivationManager: {e}"))?;
        let wide_aumid = wide(aumid);
        let args = activation_args(aumid);
        let args_wide = wide(&args);
        let args_pc = if args.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(args_wide.as_ptr())
        };
        aam.ActivateApplication(PCWSTR(wide_aumid.as_ptr()), args_pc, AO_NONE)
            .map_err(|e| format!("ActivateApplication: {e}"))?;
        Ok(())
    }
}

/// Windows Terminal 的 startingDirectory 常为 null（跟随 cwd），且已有实例时
/// 新标签页拿不到激活方 cwd；用官方 -d 参数显式指定用户主目录。其他 AUMID 不传参，
/// 避免把无法解析的参数塞给未知应用。
fn activation_args(aumid: &str) -> String {
    if aumid.starts_with("Microsoft.WindowsTerminal") {
        if let Some(home) = dirs::home_dir() {
            let s = home.to_string_lossy();
            return format!("-d \"{}\"", s.trim_end_matches('\\'));
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wt_aumid_gets_dash_d_home() {
        let args = activation_args("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App");
        let home = dirs::home_dir().expect("Windows 测试机必有主目录");
        assert_eq!(args, format!("-d \"{}\"", home.display()));
    }

    #[test]
    fn non_terminal_aumid_gets_no_args() {
        assert_eq!(activation_args("Microsoft.WindowsStore_8wekyb3d8bbwe!App"), "");
        assert_eq!(activation_args(""), "");
    }

    #[test]
    fn strip_aumid_prefix() {
        assert_eq!(
            "shell:AppsFolder\\Microsoft.WindowsStore_8wekyb3d8bbwe!App"
                .strip_prefix("shell:AppsFolder\\"),
            Some("Microsoft.WindowsStore_8wekyb3d8bbwe!App")
        );
    }
}
