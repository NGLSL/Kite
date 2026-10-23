use crate::error::{win32_error, ServiceError, ServiceResult};
use crate::PIPE_NAME;
use std::fs;
use std::fs::File;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE, HLOCAL,
};
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    GetSecurityDescriptorDacl, ACL, PROTECTED_DACL_SECURITY_INFORMATION,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, SetNamedPipeHandleState,
    NAMED_PIPE_MODE, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_WAIT,
};

pub(crate) struct SecurityDescriptor {
    ptr: windows::Win32::Security::PSECURITY_DESCRIPTOR,
}
impl SecurityDescriptor {
    pub(crate) fn for_pipe() -> ServiceResult<Self> {
        // SYSTEM/Admins may administer the endpoint; interactive users can
        // connect, then the service verifies the exact client SID/session.
        Self::from_sddl("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)")
    }
    pub(crate) fn from_sddl(sddl: &str) -> ServiceResult<Self> {
        let wide = crate::service::wide_null(sddl);
        let mut ptr = windows::Win32::Security::PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut ptr,
                None,
            )
        }
        .map_err(ServiceError::from)?;
        Ok(Self { ptr })
    }
    pub(crate) fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.ptr.0
    }
}
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.ptr.is_invalid() {
            unsafe {
                windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
                    self.ptr.0,
                )));
            }
        }
    }
}

/// Try one client pipe open. Retry policy belongs to `Client::connect`, which
/// can re-check SCM state between attempts and restart a service that stopped
/// while the previous client was disconnecting.
pub(crate) fn connect_client_once() -> ServiceResult<File> {
    let name = crate::service::wide_null(PIPE_NAME);
    let access = GENERIC_READ.0 | GENERIC_WRITE.0;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            access,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    match handle {
        Ok(value) => {
            let file = unsafe { File::from_raw_handle(value.0 as _) };
            Ok(file)
        }
        Err(error) => Err(ServiceError::from(error)),
    }
}

pub(crate) fn create_server_pipe() -> ServiceResult<(File, SecurityDescriptor)> {
    let descriptor = SecurityDescriptor::for_pipe()?;
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.as_ptr(),
        bInheritHandle: false.into(),
    };
    let name = crate::service::wide_null(PIPE_NAME);
    let open = PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE;
    // NOWAIT lets the service stop promptly when SCM requests a stop while no
    // client is connected; ConnectNamedPipe is polled below.
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open,
            mode,
            1,
            1024 * 1024,
            1024 * 1024,
            1000,
            Some(&attrs),
        )
    };
    if handle.is_invalid() {
        return Err(ServiceError::from(windows::core::Error::from_win32()));
    }
    let file = unsafe { File::from_raw_handle(handle.0 as _) };
    Ok((file, descriptor))
}

pub(crate) fn wait_for_client(
    file: &File,
    timeout: Duration,
    should_stop: impl Fn() -> bool,
) -> ServiceResult<bool> {
    let handle = HANDLE(file.as_raw_handle() as _);
    let started = Instant::now();
    loop {
        if should_stop() || started.elapsed() >= timeout {
            return Ok(false);
        }
        let result = unsafe { ConnectNamedPipe(handle, None) };
        match result {
            Ok(()) => {
                set_blocking_mode(handle)?;
                return Ok(true);
            }
            Err(error) => {
                let code = (error.code().0 as u32) & 0xffff;
                if code == ERROR_PIPE_CONNECTED.0 {
                    set_blocking_mode(handle)?;
                    return Ok(true);
                }
                if code == ERROR_PIPE_LISTENING.0 || code == ERROR_NO_DATA.0 {
                    // Keep this same named-pipe instance alive while a client
                    // is trying to connect. Recreating it on every poll leaves
                    // a short race window in which the endpoint does not
                    // exist, making a normal client miss a demand-start
                    // service intermittently.
                    if code == ERROR_NO_DATA.0 {
                        let _ = unsafe { DisconnectNamedPipe(handle) };
                    }
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                return Err(ServiceError::from(error));
            }
        }
    }
}

fn set_blocking_mode(handle: HANDLE) -> ServiceResult<()> {
    // The server uses NOWAIT only while polling for an initial client. Once
    // connected, switch the instance to blocking reads so BufReader does not
    // mistake a momentarily empty pipe for a disconnect.
    let mode: NAMED_PIPE_MODE = PIPE_READMODE_BYTE | PIPE_WAIT;
    unsafe { SetNamedPipeHandleState(handle, Some(&mode), None, None) }.map_err(ServiceError::from)
}

pub(crate) fn disconnect(file: &File) {
    let handle = HANDLE(file.as_raw_handle() as _);
    disconnect_from_handle(handle);
}

pub(crate) fn disconnect_from_handle(handle: HANDLE) {
    let _ = unsafe { DisconnectNamedPipe(handle) };
}

/// Apply a protected directory DACL.  SYSTEM owns the metadata tree; the
/// interactive user may read/list it but cannot create, replace, or delete
/// captures and feeds.
pub(crate) fn harden_metadata_directory(path: &std::path::Path, sid: &str) -> ServiceResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(ServiceError::from)?;
    if metadata.file_attributes() & 0x400 != 0 || !metadata.is_dir() {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            format!("索引服务元数据路径不是普通目录: {}", path.display()),
        ));
    }
    if sid.is_empty()
        || sid
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-'))
    {
        return Err(ServiceError::authentication(
            "invalid user SID for metadata ACL",
        ));
    }
    let descriptor = SecurityDescriptor::from_sddl(&format!(
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;{sid})"
    ))?;
    let mut present = false.into();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut defaulted = false.into();
    unsafe {
        GetSecurityDescriptorDacl(
            windows::Win32::Security::PSECURITY_DESCRIPTOR(descriptor.as_ptr()),
            &mut present,
            &mut acl,
            &mut defaulted,
        )
    }
    .map_err(ServiceError::from)?;
    if !present.as_bool() || acl.is_null() {
        return Err(ServiceError::internal("metadata directory ACL is empty"));
    }
    let wide = crate::service::wide_null(&path.to_string_lossy());
    let status = unsafe {
        SetNamedSecurityInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(acl as *const ACL),
            None,
        )
    };
    if status.0 != 0 {
        return Err(win32_error(status.0, "无法保护元数据目录 ACL"));
    }
    Ok(())
}

/// Create and lock the service-owned metadata hierarchy one component at a
/// time. Every component is checked for reparse-point substitution and, when
/// it pre-exists, for a trusted owner before its protected DACL is applied.
/// This avoids trusting a user-created `ProgramData\\Kite` junction or
/// directory before the SYSTEM service starts writing below it.
pub(crate) fn ensure_metadata_tree(program_data: &Path, sid: &str) -> ServiceResult<PathBuf> {
    if sid.is_empty()
        || sid.len() > 184
        || !sid.starts_with("S-")
        || sid
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-'))
    {
        return Err(ServiceError::authentication(
            "invalid user SID for metadata tree",
        ));
    }
    ensure_existing_directory(program_data, false)?;
    if !owner_is_os_root_trusted(program_data)? {
        return Err(ServiceError::access_denied(format!(
            "ProgramData 根目录的所有者不受信任: {}",
            program_data.display()
        )));
    }
    let kite = program_data.join("Kite");
    ensure_metadata_component(&kite, sid)?;
    let file_index = kite.join("FileIndex");
    ensure_metadata_component(&file_index, sid)?;
    let user_root = file_index.join(sid);
    ensure_metadata_component(&user_root, sid)?;
    Ok(user_root)
}

/// Validate/create the fixed Program Files host parent. The known-folder root
/// is supplied by the shell API; only the two Kite-owned descendants are
/// created and locked, so an environment-variable substitution or user-owned
/// junction cannot become the SCM binary directory.
pub(crate) fn ensure_host_directory_tree(program_files: &Path) -> ServiceResult<PathBuf> {
    ensure_existing_directory(program_files, false)?;
    if !owner_is_os_root_trusted(program_files)? {
        return Err(ServiceError::access_denied(format!(
            "Program Files 根目录的所有者不受信任: {}",
            program_files.display()
        )));
    }
    let kite = program_files.join("Kite");
    ensure_host_component(&kite)?;
    let file_index = kite.join("file-index");
    ensure_host_component(&file_index)?;
    Ok(file_index)
}

fn ensure_host_component(path: &Path) -> ServiceResult<()> {
    let existed = path.exists();
    ensure_existing_directory(path, true)?;
    if existed && !owner_is_system_or_admin(path)? {
        return Err(ServiceError::access_denied(format!(
            "索引服务安装目录的所有者不受信任: {}",
            path.display()
        )));
    }
    harden_host_directory(path)
}

fn harden_host_directory(path: &Path) -> ServiceResult<()> {
    // Program Files\Kite is also the ordinary Kite installation directory.
    // Keep the standard user read/execute surface needed to launch Kite and
    // read its resources, while the protected DACL removes user write/create
    // rights. SYSTEM and local administrators retain full control.
    let descriptor = SecurityDescriptor::from_sddl(
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)(A;OICI;GRGX;;;IU)",
    )?;
    let mut present = false.into();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut defaulted = false.into();
    unsafe {
        GetSecurityDescriptorDacl(
            windows::Win32::Security::PSECURITY_DESCRIPTOR(descriptor.as_ptr()),
            &mut present,
            &mut acl,
            &mut defaulted,
        )
    }
    .map_err(ServiceError::from)?;
    if !present.as_bool() || acl.is_null() {
        return Err(ServiceError::internal("host directory ACL is empty"));
    }
    let wide = crate::service::wide_null(&path.to_string_lossy());
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(acl as *const ACL),
            None,
        )
    };
    if status.0 != 0 {
        return Err(win32_error(status.0, "无法保护索引服务安装目录 ACL"));
    }
    Ok(())
}

/// Apply the protected owner DACL to the copied service image itself;
/// directory inheritance alone does not retroactively repair an existing
/// binary's ACL.
pub(crate) fn harden_host_file(path: &Path) -> ServiceResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(ServiceError::from)?;
    if metadata.file_attributes() & 0x400 != 0 || !metadata.is_file() {
        return Err(ServiceError::new(
            crate::ErrorKind::PathMismatch,
            format!("索引服务 host 不是普通文件: {}", path.display()),
        ));
    }
    let descriptor =
        SecurityDescriptor::from_sddl("D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;BU)(A;;GRGX;;;IU)")?;
    let mut present = false.into();
    let mut acl: *mut ACL = std::ptr::null_mut();
    let mut defaulted = false.into();
    unsafe {
        GetSecurityDescriptorDacl(
            windows::Win32::Security::PSECURITY_DESCRIPTOR(descriptor.as_ptr()),
            &mut present,
            &mut acl,
            &mut defaulted,
        )
    }
    .map_err(ServiceError::from)?;
    if !present.as_bool() || acl.is_null() {
        return Err(ServiceError::internal("host file ACL is empty"));
    }
    let wide = crate::service::wide_null(&path.to_string_lossy());
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(acl as *const ACL),
            None,
        )
    };
    if status.0 != 0 {
        return Err(win32_error(status.0, "无法保护索引服务 host ACL"));
    }
    Ok(())
}

fn ensure_metadata_component(path: &Path, sid: &str) -> ServiceResult<()> {
    let existed = path.exists();
    ensure_existing_directory(path, true)?;
    if existed && !owner_is_system_or_admin(path)? {
        return Err(ServiceError::access_denied(format!(
            "索引服务目录的所有者不受信任: {}",
            path.display()
        )));
    }
    harden_metadata_directory(path, sid)
}

fn ensure_existing_directory(path: &Path, allow_create: bool) -> ServiceResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let attributes = metadata.file_attributes();
            if attributes & 0x400 != 0 {
                return Err(ServiceError::new(
                    crate::ErrorKind::PathMismatch,
                    format!("索引服务目录不能是 reparse point: {}", path.display()),
                ));
            }
            if !metadata.is_dir() {
                return Err(ServiceError::new(
                    crate::ErrorKind::PathMismatch,
                    format!("索引服务路径不是目录: {}", path.display()),
                ));
            }
            Ok(())
        }
        Err(error) if allow_create && error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(ServiceError::from)?;
            // Re-read after creation so a concurrent replacement is rejected
            // before any ACL or metadata operation uses the path.
            ensure_existing_directory(path, false)
        }
        Err(error) => Err(ServiceError::from(error)),
    }
}

fn owner_is_system_or_admin(path: &Path) -> ServiceResult<bool> {
    let value = owner_sid(path)?;
    Ok(value == "S-1-5-18" || value == "S-1-5-32-544")
}

fn owner_is_os_root_trusted(path: &Path) -> ServiceResult<bool> {
    let value = owner_sid(path)?;
    // Program Files is normally owned by TrustedInstaller, while a locally
    // provisioned machine may report SYSTEM or Administrators. The service
    // SID namespace is accepted only for this OS-owned known-folder root;
    // Kite-owned descendants remain restricted to SYSTEM/Administrators.
    Ok(value == "S-1-5-18" || value == "S-1-5-32-544" || value.starts_with("S-1-5-80-"))
}

fn owner_sid(path: &Path) -> ServiceResult<String> {
    let wide = crate::service::wide_null(&path.to_string_lossy());
    let mut owner = PSID::default();
    let mut descriptor = windows::Win32::Security::PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut descriptor,
        )
    };
    if status.0 != 0 {
        return Err(win32_error(status.0, "无法验证索引服务目录所有者"));
    }
    let _descriptor_guard = LocalSecurityDescriptor(descriptor);
    if owner.is_invalid() {
        return Err(ServiceError::access_denied("索引服务目录没有所有者 SID"));
    }
    let mut sid_string = windows::core::PWSTR::null();
    unsafe { ConvertSidToStringSidW(owner, &mut sid_string) }.map_err(ServiceError::from)?;
    if sid_string.is_null() {
        return Err(ServiceError::access_denied("索引服务目录所有者 SID 为空"));
    }
    let value = unsafe { sid_string.to_string() }
        .map_err(|error| ServiceError::internal(error.to_string()))?;
    unsafe {
        windows::Win32::Foundation::LocalFree(Some(HLOCAL(sid_string.0 as _)));
    }
    Ok(value)
}

struct LocalSecurityDescriptor(windows::Win32::Security::PSECURITY_DESCRIPTOR);
impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                windows::Win32::Foundation::LocalFree(Some(HLOCAL(self.0 .0)));
            }
        }
    }
}
