//! 解析 Windows .lnk 快捷方式为可启动目标。
//! 优先走系统 IShellLink COM（与资源管理器一致，能处理广告/MSI 快捷方式）。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, IPersistFile,
    STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// 将 .lnk 解析为 (target, args, working_dir, icon_path)。
/// 单文件失败/panic 不拖垮整次扫描。
pub fn resolve_lnk(path: &Path) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse_lnk(path)));
    match result {
        Ok(v) => v,
        Err(_) => {
            crate::log::info(&format!("skip bad lnk: {}", path.display()));
            None
        }
    }
}

fn parse_lnk(
    path: &Path,
) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
    unsafe {
        // 扫描线程可能未初始化 COM；重复调用返回 S_FALSE / RPC_E_CHANGED_MODE 可忽略
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        persist.Load(PCWSTR(wide_path.as_ptr()), STGM_READ).ok()?;

        let mut target_buf = [0u16; 32_768];
        link.GetPath(&mut target_buf, std::ptr::null_mut(), 0).ok()?;
        let target = wide_to_string(&target_buf);
        if target.is_empty() {
            return None;
        }

        let mut args_buf = [0u16; 4096];
        let _ = link.GetArguments(&mut args_buf);
        let args = non_empty(wide_to_string(&args_buf));

        let mut wd_buf = [0u16; 1024];
        let _ = link.GetWorkingDirectory(&mut wd_buf);
        let working_dir = non_empty(wide_to_string(&wd_buf));

        // 图标源优先 .lnk 的 icon_location；为空则退回 target
        let mut icon_buf = [0u16; 1024];
        let mut icon_idx = 0i32;
        let _ = link.GetIconLocation(&mut icon_buf, &mut icon_idx);
        let icon_raw = non_empty(wide_to_string(&icon_buf));
        let icon = icon_raw.or_else(|| Some(target.clone()));

        Some((target, args, working_dir, icon))
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().to_string()
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}
