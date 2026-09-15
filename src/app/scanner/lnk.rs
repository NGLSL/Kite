//! 解析 Windows .lnk 快捷方式为可启动目标。
//! 优先走系统 IShellLink COM（与资源管理器一致，能处理广告/MSI 快捷方式）。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// 将 .lnk 解析为 (target, args, working_dir, icon_src)。
/// `icon_src` 为 `path,index`（index 可为负资源 ID）；路径为空时退回 target。
/// 单文件失败/panic 不拖垮整次扫描。
pub fn resolve_lnk(
    path: &Path,
) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse_lnk(path)));
    match result {
        Ok(v) => v,
        Err(_) => {
            crate::log::info(&format!("skip bad lnk: {}", path.display()));
            None
        }
    }
}

fn parse_lnk(path: &Path) -> Option<(String, Option<String>, Option<String>, Option<String>)> {
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
        link.GetPath(&mut target_buf, std::ptr::null_mut(), 0)
            .ok()?;
        let target_raw = wide_to_string(&target_buf);

        let mut args_buf = [0u16; 4096];
        let _ = link.GetArguments(&mut args_buf);
        let args = non_empty(wide_to_string(&args_buf));

        let mut wd_buf = [0u16; 1024];
        let _ = link.GetWorkingDirectory(&mut wd_buf);
        let working_dir = non_empty(wide_to_string(&wd_buf));

        // 图标源优先 .lnk 的 icon_location；路径与序号一并下传（`path,index`）。
        let mut icon_buf = [0u16; 1024];
        let mut icon_idx = 0i32;
        let _ = link.GetIconLocation(&mut icon_buf, &mut icon_idx);
        let icon_raw = non_empty(wide_to_string(&icon_buf));

        // Shell 型快捷方式（如 File Explorer.lnk）GetPath 为空，只有 icon_location。
        // 用展开后的图标路径当 target，避免整条入口被丢掉。
        let target = if !target_raw.is_empty() {
            target_raw
        } else {
            icon_launch_fallback(icon_raw.as_deref())?
        };

        let icon = match icon_raw {
            Some(raw) => Some(format!("{raw},{icon_idx}")),
            None => Some(target.clone()),
        };

        Some((target, args, working_dir, icon))
    }
}

/// icon_location 路径可启动时，把它当作 target（展开环境变量并检查存在）。
fn icon_launch_fallback(icon_raw: Option<&str>) -> Option<String> {
    let raw = icon_raw?;
    let path_part = raw.rsplit_once(',').map(|(p, _)| p).unwrap_or(raw);
    let expanded = crate::system::env::expand_env(path_part);
    if expanded.is_empty() {
        return None;
    }
    if Path::new(&expanded).exists() {
        Some(expanded)
    } else {
        None
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().to_string()
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_launch_fallback_expands_windir_and_strips_index() {
        let got = icon_launch_fallback(Some(r"%windir%\explorer.exe,0")).expect("explorer exists");
        assert!(got.to_lowercase().ends_with("explorer.exe"), "got={got}");
        assert!(!got.contains(','), "不应把资源序号写进 target");
    }

    #[test]
    fn icon_launch_fallback_rejects_missing_path() {
        assert!(icon_launch_fallback(Some(r"C:\definitely\not\here.exe,1")).is_none());
        assert!(icon_launch_fallback(None).is_none());
    }
}
