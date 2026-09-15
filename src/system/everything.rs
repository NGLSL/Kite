//! Everything 文件搜索：加载官方 SDK DLL（Everything64.dll）与运行中的 Everything 实例 IPC。
//! 不自建索引，也绝不拉起 Everything 主窗口；DLL 缺失或 Everything 未运行时静默返回空。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

/// 文件命中。
#[derive(Debug, Clone)]
pub struct EverythingHit {
    pub path: String,
    pub is_folder: bool,
    pub name: String,
}

const REQUEST_FILE_NAME: u32 = 0x0000_0001;
const REQUEST_PATH: u32 = 0x0000_0002;
/// Everything.h：ERROR_IPC = Everything search client is not running。
const ERROR_IPC: u32 = 2;
const DLL_NAME: &str = "Everything64.dll";
const IPC_WINDOW_CLASS: windows::core::PCWSTR =
    windows::core::w!("EVERYTHING_TASKBAR_NOTIFICATION");

type SetSearchW = unsafe extern "system" fn(*const u16);
type SetRequestFlags = unsafe extern "system" fn(u32);
type SetMax = unsafe extern "system" fn(u32);
type QueryW = unsafe extern "system" fn(i32) -> i32;
type GetLastError = unsafe extern "system" fn() -> u32;
type GetNumResults = unsafe extern "system" fn() -> u32;
type GetResultPathW = unsafe extern "system" fn(u32) -> *const u16;
type GetResultFileNameW = unsafe extern "system" fn(u32) -> *const u16;
type IsFolderResult = unsafe extern "system" fn(u32) -> i32;

/// SDK DLL 的进程级全局状态非线程安全，查询串行化。
static QUERY_LOCK: Mutex<()> = Mutex::new(());
static SDK: OnceLock<Option<Sdk>> = OnceLock::new();
static IPC_MISS_LOGGED: AtomicBool = AtomicBool::new(false);

struct Sdk {
    set_search: SetSearchW,
    set_request_flags: SetRequestFlags,
    set_max: SetMax,
    query: QueryW,
    get_last_error: GetLastError,
    get_num_results: GetNumResults,
    get_result_path: GetResultPathW,
    get_result_name: GetResultFileNameW,
    is_folder: IsFolderResult,
}

fn sdk() -> Option<&'static Sdk> {
    SDK.get_or_init(|| unsafe { load_sdk() }).as_ref()
}

fn everything_ipc_window_available() -> bool {
    unsafe { FindWindowW(IPC_WINDOW_CLASS, windows::core::PCWSTR::null()).is_ok() }
}

unsafe fn load_sdk() -> Option<Sdk> {
    // 捆绑路径 + Everything 安装目录 + 系统 PATH
    let mut cands = crate::system::resources::candidates(DLL_NAME);
    cands.extend(install_dirs().into_iter().map(|d| d.join(DLL_NAME)));
    cands.push(PathBuf::from(DLL_NAME));

    let mut last_err = None;
    for cand in cands {
        let wide: Vec<u16> = OsStr::new(cand.as_os_str())
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        match LoadLibraryW(windows::core::PCWSTR(wide.as_ptr())) {
            Ok(h) => {
                if let Some(sdk) = build_sdk(h) {
                    return Some(sdk);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    crate::log::info(&format!(
        "Everything64.dll not found; file search disabled. last_err={last_err:?}"
    ));
    None
}

unsafe fn build_sdk(h: HMODULE) -> Option<Sdk> {
    unsafe fn sym<T>(h: HMODULE, name: &[u8]) -> Option<T> {
        let far = GetProcAddress(h, windows::core::PCSTR::from_raw(name.as_ptr()));
        if far.is_none() {
            return None;
        }
        // FARPROC 与函数指针同为 8 字节（Option<fn> 的 niche 优化）
        Some(std::mem::transmute_copy(&far))
    }
    Some(Sdk {
        set_search: sym(h, b"Everything_SetSearchW\0")?,
        set_request_flags: sym(h, b"Everything_SetRequestFlags\0")?,
        set_max: sym(h, b"Everything_SetMax\0")?,
        query: sym(h, b"Everything_QueryW\0")?,
        get_last_error: sym(h, b"Everything_GetLastError\0")?,
        get_num_results: sym(h, b"Everything_GetNumResults\0")?,
        get_result_path: sym(h, b"Everything_GetResultPathW\0")?,
        get_result_name: sym(h, b"Everything_GetResultFileNameW\0")?,
        is_folder: sym(h, b"Everything_IsFolderResult\0")?,
    })
}

/// Everything 安装目录（SDK DLL 也常被用户放在这里；不存在则跳过）。
fn install_dirs() -> Vec<PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"D:\Program Files\Everything",
        r"C:\Program Files\Everything",
        r"C:\Program Files (x86)\Everything",
    ];
    let mut v: Vec<PathBuf> = CANDIDATES
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.join("Everything.exe").exists())
        .collect();
    if let Ok(out) = std::process::Command::new("where")
        .arg("Everything.exe")
        .output()
    {
        if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
            let exe = PathBuf::from(line.trim());
            if let Some(dir) = exe.parent() {
                v.push(dir.to_path_buf());
            }
        }
    }
    v
}

/// 查询 Everything；`max` 为返回上限。未运行/未安装返回空，不产生任何窗口。
pub fn search_files(query: &str, max: usize) -> Vec<EverythingHit> {
    let q = query.trim();
    if q.is_empty() || max == 0 {
        return Vec::new();
    }
    // SDK 本质是 Everything IPC 包装。官方 IPC 示例同样先 FindWindow；
    // 没有服务端窗口时绝不能调用可能等待数秒的阻塞查询。
    if !everything_ipc_window_available() {
        if !IPC_MISS_LOGGED.swap(true, Ordering::Relaxed) {
            crate::log::info("everything not running; file search skipped");
        }
        return Vec::new();
    }
    let Some(sdk) = sdk() else {
        return Vec::new();
    };
    let _guard = QUERY_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    unsafe {
        let wide: Vec<u16> = OsStr::new(q)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        (sdk.set_search)(wide.as_ptr());
        (sdk.set_request_flags)(REQUEST_FILE_NAME | REQUEST_PATH);
        (sdk.set_max)(max as u32);

        if (sdk.query)(1) == 0 {
            let err = (sdk.get_last_error)();
            if err == ERROR_IPC && !IPC_MISS_LOGGED.swap(true, Ordering::Relaxed) {
                crate::log::info("everything not running; file search skipped");
            }
            return Vec::new();
        }

        let n = (sdk.get_num_results)().min(max as u32);
        let mut hits = Vec::with_capacity(n as usize);
        for i in 0..n {
            let path_ptr = (sdk.get_result_path)(i);
            let name_ptr = (sdk.get_result_name)(i);
            if path_ptr.is_null() || name_ptr.is_null() {
                continue;
            }
            let dir = ptr_to_string(path_ptr);
            let name = ptr_to_string(name_ptr);
            if name.is_empty() {
                continue;
            }
            let full = format!("{}\\{name}", dir.trim_end_matches(['\\', '/']));
            hits.push(EverythingHit {
                path: full,
                is_folder: (sdk.is_folder)(i) != 0,
                name,
            });
        }
        hits
    }
}

/// SDK 返回的 LPCWSTR 仅在下一次查询前有效，立即拷贝。
unsafe fn ptr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    #[test]
    fn empty_query_short_circuits() {
        // 不触碰 DLL：空查询/零上限直接返回空
        assert!(search_files("", 5).is_empty());
        assert!(search_files("   ", 5).is_empty());
        assert!(search_files("report", 0).is_empty());
    }

    #[test]
    fn install_dirs_no_crash_without_everything() {
        // 探测逻辑在任何机器上都应安全返回
        let dirs = install_dirs();
        let _ = dirs.len();
    }

    #[test]
    fn dll_candidates_have_expected_shape() {
        let mut cands = crate::system::resources::candidates(DLL_NAME);
        cands.extend(install_dirs().into_iter().map(|d| d.join(DLL_NAME)));
        cands.push(PathBuf::from(DLL_NAME));
        assert!(cands.iter().all(|p| p.file_name().unwrap() == DLL_NAME));
        // 末个交给系统搜索顺序
        assert_eq!(cands.last().unwrap().parent(), Some(Path::new("")));
        // 含编译期 crate 目录兜底
        assert!(cands
            .iter()
            .any(|p| p.starts_with(env!("CARGO_MANIFEST_DIR"))));
    }

    /// 真实 IPC 冒烟：按与运行时一致的布局加载仓库内捆绑的 DLL 查询一次。
    /// Everything 未运行的机器上查询结果为空也算通过；DLL 定位/加载失败则不通过。
    #[test]
    fn sdk_smoke_via_bundled_dll() {
        crate::system::resources::init(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        assert!(locate_dll().is_some(), "捆绑 DLL 应可定位");
        assert!(sdk().is_some(), "Everything64.dll 应能加载并解析符号");
        let hits = search_files("Everything.exe", 5);
        assert!(hits.len() <= 5);
        for h in &hits {
            assert!(!h.name.is_empty(), "结果名不应为空");
            assert!(!h.path.is_empty(), "结果路径不应为空");
        }
        // 本机 Everything 在运行时应有命中;此处仅打印,避免强依赖外部进程
        eprintln!("sdk smoke hits={}", hits.len());
    }

    #[test]
    fn missing_everything_does_not_wait_for_ipc() {
        // 回归检查引擎未运行时不会进入 SDK 的多秒 IPC 等待；
        // 引擎正在运行的机器跳过此环境测试。
        if everything_ipc_window_available() {
            return;
        }
        crate::system::resources::init(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let started = Instant::now();
        let _ = search_files("we", 5);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "missing Everything probe took {:?}",
            started.elapsed()
        );
    }

    fn locate_dll() -> Option<PathBuf> {
        crate::system::resources::locate(DLL_NAME)
    }
}
