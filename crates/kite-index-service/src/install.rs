use crate::error::{ErrorKind, ServiceError, ServiceResult};
use crate::pipe;
use crate::{SERVICE_DISPLAY_NAME, SERVICE_NAME};
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SERVICE_DOES_NOT_EXIST};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, CloseServiceHandle, ControlService, CreateServiceW, DeleteService,
    OpenSCManagerW, OpenServiceW, QueryServiceConfigW, QueryServiceStatusEx,
    SetServiceObjectSecurity, StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_CREATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_CONFIG_DESCRIPTION,
    SERVICE_CONTROL_STOP, SERVICE_DEMAND_START, SERVICE_ERROR_NORMAL, SERVICE_QUERY_CONFIG,
    SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_START_PENDING, SERVICE_STATUS,
    SERVICE_STATUS_PROCESS, SERVICE_STOP, SERVICE_STOPPED, SERVICE_STOP_PENDING,
    SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramFilesX64, SHGetKnownFolderPath, ShellExecuteExW, KF_FLAG_DEFAULT,
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Missing,
    Stopped,
    Starting,
    Running,
    Stopping,
    PathMismatch,
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// Source is the currently running Kite executable.  It is copied into
    /// the fixed protected Program Files location before SCM registration.
    pub executable: PathBuf,
}

pub(crate) fn fixed_host_path() -> ServiceResult<PathBuf> {
    let root = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFilesX64, KF_FLAG_DEFAULT, None) }
        .map_err(ServiceError::from)?;
    let value =
        unsafe { root.to_string() }.map_err(|error| ServiceError::internal(error.to_string()));
    unsafe {
        CoTaskMemFree(Some(root.0 as _));
    }
    let root = value?;
    Ok(PathBuf::from(root)
        .join("Kite")
        .join("file-index")
        .join("kite-index-host.exe"))
}

pub fn current_state() -> ServiceResult<ServiceState> {
    let scm =
        unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }.map_err(ServiceError::from)?;
    let _scm_guard = ServiceHandle(scm);
    let service = match unsafe {
        OpenServiceW(
            scm,
            PCWSTR(crate::service::wide_null(SERVICE_NAME).as_ptr()),
            SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG,
        )
    } {
        Ok(value) => value,
        Err(error) if (error.code().0 as u32 & 0xffff) == ERROR_SERVICE_DOES_NOT_EXIST.0 => {
            return Ok(ServiceState::Missing)
        }
        Err(error) => return Err(ServiceError::from(error)),
    };
    let _service_guard = ServiceHandle(service);
    let expected = fixed_host_path()?;
    if let Some(path) = query_binary_path(service)? {
        if normalize_path(&path) != normalize_path(&expected) {
            return Ok(ServiceState::PathMismatch);
        }
    }
    let status = query_status(service)?;
    Ok(match status.dwCurrentState {
        SERVICE_RUNNING => ServiceState::Running,
        SERVICE_START_PENDING => ServiceState::Starting,
        SERVICE_STOP_PENDING => ServiceState::Stopping,
        _ => ServiceState::Stopped,
    })
}

pub fn ensure_installed(options: &InstallOptions) -> ServiceResult<ServiceState> {
    install_entry(options)?;
    current_state()
}

/// Request the one explicit first-use elevation. The unelevated Kite process
/// only starts the same executable with the internal install flag; it never
/// receives a service handle or writes Program Files itself. The call waits
/// for the elevated child and reports its exit status so the UI can
/// distinguish an accepted install, a declined UAC prompt, and a real error.
pub fn request_install() -> ServiceResult<()> {
    let executable = std::env::current_exe().map_err(ServiceError::from)?;
    let verb = crate::service::wide_null("runas");
    let file = crate::service::wide_null(&executable.to_string_lossy());
    let parameters = crate::service::wide_null("--kite-index-install");
    let mut info = SHELLEXECUTEINFOW::default();
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = PCWSTR(verb.as_ptr());
    info.lpFile = PCWSTR(file.as_ptr());
    info.lpParameters = PCWSTR(parameters.as_ptr());
    // SW_HIDE. The elevated helper has no UI; the consent dialog remains
    // visible because it is owned by the shell rather than this child.
    info.nShow = 0;
    unsafe { ShellExecuteExW(&mut info) }.map_err(map_elevation_error)?;
    let process = info.hProcess;
    if process.is_invalid() {
        return Err(ServiceError::internal("管理员安装进程未返回句柄"));
    }
    let wait = unsafe { WaitForSingleObject(process, u32::MAX) };
    if wait.0 != 0 {
        let _ = unsafe { CloseHandle(process) };
        return Err(ServiceError::service("等待管理员安装进程结束失败"));
    }
    let mut exit_code = 1u32;
    let result = unsafe { GetExitCodeProcess(process, &mut exit_code) };
    let close_result = unsafe { CloseHandle(process) };
    result.map_err(ServiceError::from)?;
    close_result.map_err(ServiceError::from)?;
    if exit_code == 0 {
        Ok(())
    } else {
        Err(ServiceError::service(format!(
            "管理员安装进程退出，代码 {exit_code}"
        )))
    }
}

fn map_elevation_error(error: windows::core::Error) -> ServiceError {
    let code = error.code().0 as u32 & 0xffff;
    if code == 1223 {
        ServiceError::with_code(ErrorKind::Cancelled, "用户取消了管理员授权", code)
    } else {
        ServiceError::from(error)
    }
}

/// Elevated install entry.  It never invokes UAC itself; the caller should
/// launch the same executable with `runas` after showing the first-use prompt.
pub fn install_entry(options: &InstallOptions) -> ServiceResult<()> {
    let source = options
        .executable
        .canonicalize()
        .map_err(ServiceError::from)?;
    if !source.is_file() {
        return Err(ServiceError::invalid_request("索引服务源程序不是文件"));
    }
    let target = fixed_host_path()?;
    let mut replacement_backup = None;
    let mut target_installed = false;
    let program_files = target
        .ancestors()
        .nth(3)
        .ok_or_else(|| ServiceError::internal("无法确定 Program Files 根目录"))?;
    let protected_parent = pipe::ensure_host_directory_tree(program_files)?;
    if target.exists() {
        let metadata = fs::symlink_metadata(&target).map_err(ServiceError::from)?;
        if metadata.file_attributes() & 0x400 != 0 || !metadata.is_file() {
            return Err(ServiceError::new(
                ErrorKind::PathMismatch,
                "索引服务安装目标不是受保护的普通文件",
            ));
        }
    }
    if normalize_path(&source) != normalize_path(&target) {
        let parent = protected_parent;
        let temp = parent.join("kite-index-host.exe.new");
        if temp.exists() {
            let _ = fs::remove_file(&temp);
        }
        fs::copy(&source, &temp).map_err(ServiceError::from)?;
        let metadata = fs::metadata(&temp).map_err(ServiceError::from)?;
        if metadata.len() == 0 {
            let _ = fs::remove_file(&temp);
            return Err(ServiceError::internal("索引服务副本为空"));
        }
        if target.exists() {
            // Keep the previous protected image until the new image has been
            // moved into place. This avoids losing a usable service binary
            // when the replacement is interrupted or the target is locked.
            let backup = parent.join("kite-index-host.exe.old");
            if backup.exists() {
                fs::remove_file(&backup).map_err(ServiceError::from)?;
            }
            fs::rename(&target, &backup).map_err(ServiceError::from)?;
            if let Err(error) = fs::rename(&temp, &target) {
                let _ = fs::rename(&backup, &target);
                let _ = fs::remove_file(&temp);
                return Err(ServiceError::from(error));
            }
            replacement_backup = Some(backup);
            target_installed = true;
        } else {
            fs::rename(&temp, &target).map_err(ServiceError::from)?;
            target_installed = true;
        }
    }
    if let Err(error) = pipe::harden_host_file(&target) {
        rollback_install_image(&target, replacement_backup.as_deref(), target_installed);
        return Err(error);
    }
    let target_string = quoted_service_command(&target);
    let scm =
        match unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE) }
        {
            Ok(value) => value,
            Err(error) => {
                rollback_install_image(&target, replacement_backup.as_deref(), target_installed);
                return Err(ServiceError::from(error));
            }
        };
    let _scm_guard = ServiceHandle(scm);
    let service = match unsafe {
        CreateServiceW(
            scm,
            PCWSTR(crate::service::wide_null(SERVICE_NAME).as_ptr()),
            PCWSTR(crate::service::wide_null(SERVICE_DISPLAY_NAME).as_ptr()),
            SERVICE_QUERY_STATUS
                | SERVICE_QUERY_CONFIG
                | SERVICE_START
                | SERVICE_STOP
                | 0x0001_0000u32
                | 0x0004_0000u32,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            PCWSTR(target_string.as_ptr()),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    } {
        Ok(value) => value,
        Err(error) if (error.code().0 as u32 & 0xffff) == 1073 => {
            let existing = match unsafe {
                OpenServiceW(
                    scm,
                    PCWSTR(crate::service::wide_null(SERVICE_NAME).as_ptr()),
                    SERVICE_QUERY_STATUS
                        | SERVICE_QUERY_CONFIG
                        | SERVICE_START
                        | SERVICE_STOP
                        | 0x0004_0000u32,
                )
            } {
                Ok(value) => value,
                Err(error) => {
                    rollback_install_image(
                        &target,
                        replacement_backup.as_deref(),
                        target_installed,
                    );
                    return Err(ServiceError::from(error));
                }
            };
            let actual = match query_binary_path(existing) {
                Ok(value) => value,
                Err(error) => {
                    let _ = unsafe { CloseServiceHandle(existing) };
                    rollback_install_image(
                        &target,
                        replacement_backup.as_deref(),
                        target_installed,
                    );
                    return Err(error);
                }
            };
            if actual.as_deref().map(normalize_path).as_deref()
                != Some(normalize_path(&target).as_path())
            {
                let _ = unsafe { CloseServiceHandle(existing) };
                rollback_install_image(&target, replacement_backup.as_deref(), target_installed);
                return Err(ServiceError::new(
                    ErrorKind::PathMismatch,
                    "已存在的 Kite 文件索引服务指向非受保护路径",
                ));
            }
            match apply_service_security(existing) {
                Ok(()) => {
                    let _ = unsafe { CloseServiceHandle(existing) };
                    if let Some(backup) = replacement_backup.take() {
                        let _ = fs::remove_file(backup);
                    }
                    return Ok(());
                }
                Err(error) => {
                    let _ = unsafe { CloseServiceHandle(existing) };
                    rollback_install_image(
                        &target,
                        replacement_backup.as_deref(),
                        target_installed,
                    );
                    return Err(error);
                }
            }
        }
        Err(error) => {
            rollback_install_image(&target, replacement_backup.as_deref(), target_installed);
            return Err(ServiceError::from(error));
        }
    };
    let _service_guard = ServiceHandle(service);
    if let Err(error) = apply_service_security(service) {
        let _ = unsafe { DeleteService(service) };
        drop(_service_guard);
        rollback_install_image(&target, replacement_backup.as_deref(), target_installed);
        return Err(error);
    }
    let description = crate::service::wide_null("仅按需读取 Kite 文件名和 USN 元数据");
    let desc = windows::Win32::System::Services::SERVICE_DESCRIPTIONW {
        lpDescription: PWSTR(description.as_ptr() as _),
    };
    let _ = unsafe {
        ChangeServiceConfig2W(
            service,
            SERVICE_CONFIG_DESCRIPTION,
            Some((&desc as *const windows::Win32::System::Services::SERVICE_DESCRIPTIONW).cast()),
        )
    };
    if let Some(backup) = replacement_backup {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

fn apply_service_security(service: SC_HANDLE) -> ServiceResult<()> {
    // SYSTEM and local administrators retain full SCM control. Interactive
    // users receive only query-config/query-status/start (CC|LC|RP); no
    // SERVICE_CHANGE_CONFIG or service stop/control right is delegated.
    let descriptor = pipe::SecurityDescriptor::from_sddl(
        "D:P(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCRP;;;IU)",
    )?;
    unsafe {
        SetServiceObjectSecurity(
            service,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR(descriptor.as_ptr()),
        )
    }
    .map_err(ServiceError::from)
}

fn rollback_install_image(target: &Path, backup: Option<&Path>, target_installed: bool) {
    if let Some(backup) = backup {
        if target.exists() {
            let _ = fs::remove_file(target);
        }
        let _ = fs::rename(backup, target);
    } else if target_installed {
        let _ = fs::remove_file(target);
    }
}

pub fn uninstall_entry() -> ServiceResult<()> {
    let scm =
        unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }.map_err(ServiceError::from)?;
    let _scm_guard = ServiceHandle(scm);
    let service = match unsafe {
        OpenServiceW(
            scm,
            PCWSTR(crate::service::wide_null(SERVICE_NAME).as_ptr()),
            SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_STOP | 0x0001_0000u32,
        )
    } {
        Ok(value) => value,
        Err(error) if (error.code().0 as u32 & 0xffff) == ERROR_SERVICE_DOES_NOT_EXIST.0 => {
            return Ok(())
        }
        Err(error) => return Err(ServiceError::from(error)),
    };
    let _service_guard = ServiceHandle(service);
    verify_service_path(service)?;
    stop_handle(service)?;
    unsafe { DeleteService(service) }.map_err(ServiceError::from)?;
    if let Ok(target) = fixed_host_path() {
        if target.exists() {
            let _ = fs::remove_file(target);
        }
    }
    Ok(())
}

/// Internal installer/update flag: stop only Kite's own service and return.
/// It deliberately does not terminate the service process or any unrelated
/// process; SCM performs the normal stop handshake.
pub(crate) fn stop_for_update_entry() -> ServiceResult<()> {
    stop_service()
}

/// Ensure the fixed service reaches RUNNING before the caller attempts to
/// open its pipe. The service can still be stopping when a previous Kite
/// client disconnects, so a single `StartServiceW` call is not sufficient:
/// wait for STOP_PENDING to settle, then start again if it reaches STOPPED.
/// The deadline is supplied by the connection caller so a stop/start race
/// cannot give each retry a fresh timeout.
pub(crate) fn start_service_until(deadline: Instant) -> ServiceResult<()> {
    let scm =
        unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }.map_err(ServiceError::from)?;
    let _scm_guard = ServiceHandle(scm);
    let service = unsafe {
        OpenServiceW(
            scm,
            PCWSTR(crate::service::wide_null(SERVICE_NAME).as_ptr()),
            SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_START,
        )
    }
    .map_err(ServiceError::from)?;
    let _service_guard = ServiceHandle(service);
    verify_service_path(service)?;
    loop {
        let state = query_status(service)?.dwCurrentState;
        if state == SERVICE_RUNNING {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(ServiceError::service("等待 Kite 文件索引服务启动超时"));
        }
        if state == SERVICE_STOPPED {
            match unsafe { StartServiceW(service, None) } {
                Ok(()) => {}
                // The status query and StartServiceW are separate calls. If
                // another owner won that small race, continue observing the
                // same service handle instead of turning it into an error.
                Err(error) if (error.code().0 as u32 & 0xffff) == 1056 => {}
                Err(error) => return Err(ServiceError::from(error)),
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn stop_service() -> ServiceResult<()> {
    let scm =
        unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }.map_err(ServiceError::from)?;
    let _scm_guard = ServiceHandle(scm);
    let service = match unsafe {
        OpenServiceW(
            scm,
            PCWSTR(crate::service::wide_null(SERVICE_NAME).as_ptr()),
            SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_STOP,
        )
    } {
        Ok(value) => value,
        Err(error) if (error.code().0 as u32 & 0xffff) == ERROR_SERVICE_DOES_NOT_EXIST.0 => {
            return Ok(())
        }
        Err(error) => return Err(ServiceError::from(error)),
    };
    let _service_guard = ServiceHandle(service);
    verify_service_path(service)?;
    stop_handle(service)
}

fn verify_service_path(service: SC_HANDLE) -> ServiceResult<()> {
    let expected = fixed_host_path()?;
    let actual = query_binary_path(service)?;
    if actual.as_deref().map(normalize_path).as_deref() != Some(normalize_path(&expected).as_path())
    {
        return Err(ServiceError::new(
            ErrorKind::PathMismatch,
            "Kite 文件索引服务未指向受保护的 Program Files 路径",
        ));
    }
    Ok(())
}

fn stop_handle(service: SC_HANDLE) -> ServiceResult<()> {
    let mut status = SERVICE_STATUS::default();
    match unsafe { ControlService(service, SERVICE_CONTROL_STOP, &mut status) } {
        Ok(()) => {}
        Err(error) if (error.code().0 as u32 & 0xffff) == 1062 => return Ok(()),
        Err(error) => return Err(ServiceError::from(error)),
    }
    wait_for_state(
        service,
        windows::Win32::System::Services::SERVICE_STOPPED,
        Duration::from_secs(8),
    )
}

fn wait_for_state(
    service: SC_HANDLE,
    desired: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    timeout: Duration,
) -> ServiceResult<()> {
    let begin = Instant::now();
    while begin.elapsed() < timeout {
        let state = query_status(service)?.dwCurrentState;
        if state == desired {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(ServiceError::service("等待 Kite 文件索引服务状态超时"))
}

fn query_status(service: SC_HANDLE) -> ServiceResult<SERVICE_STATUS_PROCESS> {
    let mut bytes = [0u8; std::mem::size_of::<SERVICE_STATUS_PROCESS>()];
    let mut needed = 0u32;
    unsafe {
        QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            Some(&mut bytes),
            &mut needed,
        )
    }
    .map_err(ServiceError::from)?;
    Ok(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const SERVICE_STATUS_PROCESS) })
}

fn query_binary_path(service: SC_HANDLE) -> ServiceResult<Option<PathBuf>> {
    let mut needed = 0u32;
    let first = unsafe { QueryServiceConfigW(service, None, 0, &mut needed) };
    if first.is_ok() || needed == 0 {
        return Ok(None);
    }
    let mut bytes = vec![0u8; needed as usize];
    unsafe {
        QueryServiceConfigW(
            service,
            Some(bytes.as_mut_ptr() as *mut windows::Win32::System::Services::QUERY_SERVICE_CONFIGW),
            bytes.len() as u32,
            &mut needed,
        )
    }
    .map_err(ServiceError::from)?;
    let config = unsafe {
        &*(bytes.as_ptr() as *const windows::Win32::System::Services::QUERY_SERVICE_CONFIGW)
    };
    if config.lpBinaryPathName.is_null() {
        return Ok(None);
    }
    let raw = unsafe { config.lpBinaryPathName.to_string() }
        .map_err(|error| ServiceError::internal(error.to_string()))?;
    Ok(parse_command_path(&raw))
}

fn parse_command_path(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(PathBuf::from(&rest[..end]));
    }
    Some(PathBuf::from(trimmed.split_whitespace().next()?))
}

fn quoted_service_command(path: &Path) -> Vec<u16> {
    let value = format!("\"{}\" --kite-index-service", path.display());
    OsStr::new(&value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn normalize_path(path: &Path) -> PathBuf {
    PathBuf::from(
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_ascii_lowercase(),
    )
}

struct ServiceHandle(SC_HANDLE);
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}
