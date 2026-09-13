//! 唤起音效：Kite 窗口被唤起（快捷键/托盘，统一经 window::show_and_focus）时
//! 播放捆绑的 `resources/open.wav`。
//! 用 winmm PlaySound 异步播放，任何失败静默（缺文件不放系统提示音）。

use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};

const WAV_NAME: &str = "open.wav";
/// 编码好的 wav 路径（UTF-16，NUL 结尾）。`SND_ASYNC` 下 winmm 只存指针，
/// 必须用进程级常量保活，不能传局部缓冲。
static SND_PATH_WIDE: OnceLock<Option<Vec<u16>>> = OnceLock::new();

/// 窗口被唤起时调用（show_and_focus）。`SND_ASYNC` 立即返回，绝不拖慢显示路径。
pub fn play_open() {
    let Some(wide) = SND_PATH_WIDE.get_or_init(resolve_wide) else {
        return;
    };
    // SND_NODEFAULT：文件缺失/解码失败时不顶替系统默认音;返回值无意义,忽略
    unsafe {
        let _ = PlaySoundW(
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            SND_ASYNC | SND_FILENAME | SND_NODEFAULT,
        );
    }
}

/// 解析一次并编码；找不到时静默禁用（记一次日志）。
fn resolve_wide() -> Option<Vec<u16>> {
    static MISS_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(path) = crate::system::resources::locate(WAV_NAME) else {
        if !MISS_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            crate::log::info("open.wav not found; open sound disabled");
        }
        return None;
    };
    Some(
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_yields_bundled_file() {
        crate::system::resources::init(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let wide = resolve_wide();
        assert!(wide.is_some(), "应解析到 open.wav");
        assert_eq!(wide.unwrap().last(), Some(&0), "UTF-16 路径应以 NUL 结尾");
    }
}
