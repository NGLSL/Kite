//! 单例：命名 Event + 命名 Mutex。二次启动通过 Event 请求主实例显示窗口。
//! 必须在降权之后 claim，避免提权 bootstrap 短暂占位导致正常实例被误判为二次启动。
//! 先创建 Event 再创建 Mutex：二次启动看到 Mutex 已存在时，Event 必已可 Open。

use std::sync::OnceLock;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
    INFINITE,
};

const MUTEX_NAME: &str = r"Local\com.kite.launcher.instance";
const EVENT_NAME: &str = r"Local\com.kite.launcher.activate";

/// 二次启动：主实例已在运行，激活请求已尽力发出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyRunning;

/// claim 内核对象创建失败（权限/资源等），与「已有实例」区分。
#[derive(Debug)]
pub struct CreateFailed(pub String);

/// 主实例持有的内核对象；Drop 时关闭句柄（正常退出也可依赖进程清理）。
pub struct Instance {
    mutex: HANDLE,
    event: HANDLE,
}

// HANDLE 本身不实现 Send/Sync；句柄由本模块独占创建，等待/信号为系统线程安全原语。
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.event);
            let _ = CloseHandle(self.mutex);
        }
    }
}

static PRIMARY: OnceLock<Instance> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 进程级单例 claim。`Ok(())` 本进程可继续；`Err(AlreadyRunning)` 应退出。
/// 内核对象创建失败时记录日志并继续（降级为无单例保护），避免误杀唯一可用实例。
pub fn claim() -> Result<(), AlreadyRunning> {
    if PRIMARY.get().is_some() {
        return Ok(());
    }
    match claim_with(MUTEX_NAME, EVENT_NAME) {
        Ok(instance) => {
            // 同进程若已 claim（理论不应发生），丢弃本次 Instance 的额外句柄即可；
            // 命名对象在 PRIMARY 持有的句柄关闭前仍存在。
            let _ = PRIMARY.set(instance);
            Ok(())
        }
        Err(ClaimError::AlreadyRunning) => Err(AlreadyRunning),
        Err(ClaimError::CreateFailed(message)) => {
            crate::log::info(&format!(
                "singleton unavailable ({message}); continuing without single-instance guard"
            ));
            Ok(())
        }
    }
}

enum ClaimError {
    AlreadyRunning,
    CreateFailed(String),
}

/// 可指定对象名的 claim（测试与隔离场景用）。
fn claim_with(mutex_name: &str, event_name: &str) -> Result<Instance, ClaimError> {
    let mutex_w = wide(mutex_name);
    let event_w = wide(event_name);
    unsafe {
        // Event 先建：二次启动在 Mutex 已存在时总能 OpenEvent。
        let event = CreateEventW(None, false, false, PCWSTR(event_w.as_ptr())).map_err(|error| {
            let message = format!("event create failed: {error}");
            crate::log::info(&format!("singleton {message}"));
            ClaimError::CreateFailed(message)
        })?;

        let mutex = match CreateMutexW(None, true, PCWSTR(mutex_w.as_ptr())) {
            Ok(mutex) => mutex,
            Err(error) => {
                let _ = CloseHandle(event);
                let message = format!("mutex create failed: {error}");
                crate::log::info(&format!("singleton {message}"));
                return Err(ClaimError::CreateFailed(message));
            }
        };

        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            let _ = CloseHandle(event);
            signal_activate(event_w.as_ptr());
            crate::log::info("singleton claim rejected; activation requested");
            return Err(ClaimError::AlreadyRunning);
        }

        Ok(Instance { mutex, event })
    }
}

fn signal_activate(event_name: *const u16) {
    // Event 已先于 Mutex 创建；短暂重试覆盖极端调度延迟。
    for attempt in 0..20u32 {
        unsafe {
            if let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_name)) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
                return;
            }
        }
        let _ = attempt;
        std::thread::sleep(Duration::from_millis(10));
    }
    crate::log::info("singleton activation event open failed");
}

impl Instance {
    /// 主实例监听二次启动的激活请求；每次触发调用一次 `on_activate`。
    pub fn spawn_activation_listener(&self, on_activate: impl Fn() + Send + 'static) {
        // HANDLE 不是 Send；句柄值在进程内稳定，用 isize 跨线程等待。
        let event = self.event.0 as isize;
        std::thread::spawn(move || loop {
            let handle = HANDLE(event as *mut core::ffi::c_void);
            let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
            if wait == WAIT_OBJECT_0 {
                on_activate();
            } else {
                crate::log::info(&format!(
                    "singleton activation wait failed ({wait:?}); listener stopped"
                ));
                break;
            }
        });
    }
}

/// 主实例是否已 claim；供 boot 在 UI 消息桥就绪后挂监听。
pub fn spawn_activation_listener(on_activate: impl Fn() + Send + 'static) -> Result<(), String> {
    let primary = PRIMARY
        .get()
        .ok_or_else(|| "primary instance not claimed".to_string())?;
    primary.spawn_activation_listener(on_activate);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    fn unique_names(tag: &str) -> (String, String) {
        let suffix = format!("{}.{}", std::process::id(), tag);
        (
            format!(r"Local\com.kite.launcher.test.instance.{suffix}"),
            format!(r"Local\com.kite.launcher.test.activate.{suffix}"),
        )
    }

    fn expect_primary(mutex_name: &str, event_name: &str) -> Instance {
        match claim_with(mutex_name, event_name) {
            Ok(instance) => instance,
            Err(ClaimError::AlreadyRunning) => panic!("unexpected AlreadyRunning"),
            Err(ClaimError::CreateFailed(message)) => panic!("create failed: {message}"),
        }
    }

    fn expect_already_running(mutex_name: &str, event_name: &str) {
        match claim_with(mutex_name, event_name) {
            Ok(_) => panic!("expected AlreadyRunning"),
            Err(ClaimError::AlreadyRunning) => {}
            Err(ClaimError::CreateFailed(message)) => panic!("create failed: {message}"),
        }
    }

    #[test]
    fn primary_then_secondary_is_already_running() {
        let (mutex_name, event_name) = unique_names("claim");
        let _primary = expect_primary(&mutex_name, &event_name);
        expect_already_running(&mutex_name, &event_name);
    }

    #[test]
    fn dropping_primary_allows_next_claim() {
        let (mutex_name, event_name) = unique_names("drop");
        {
            let _primary = expect_primary(&mutex_name, &event_name);
        }
        let again = expect_primary(&mutex_name, &event_name);
        drop(again);
    }

    #[test]
    fn secondary_triggers_activation_listener() {
        let (mutex_name, event_name) = unique_names("activate");
        let primary = expect_primary(&mutex_name, &event_name);
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_in_thread = hits.clone();
        primary.spawn_activation_listener(move || {
            hits_in_thread.fetch_add(1, Ordering::SeqCst);
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        // 让监听线程先进入等待，再发二次 claim。
        std::thread::sleep(Duration::from_millis(30));
        expect_already_running(&mutex_name, &event_name);
        while hits.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "activation was not observed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
