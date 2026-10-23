use crate::error::{ServiceError, ServiceResult};
use std::mem::size_of;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Debug, Clone)]
pub(crate) struct ClientIdentity {
    pub pid: u32,
    pub parent_pid: u32,
    pub session_id: u32,
    pub sid: String,
}

pub(crate) fn current_client_identity(parent_pid: u32) -> ServiceResult<ClientIdentity> {
    let pid = unsafe { GetCurrentProcessId() };
    let actual_parent = process_parent_pid(pid)?;
    // Older controller call sites passed the client PID here while the
    // service protocol carries the actual parent PID. Treat that value as an
    // omitted hint, then still derive and authenticate the real parent below;
    // never trust a caller-supplied PID over the process snapshot.
    let parent_pid = if parent_pid == 0 || parent_pid == pid {
        actual_parent
    } else {
        parent_pid
    };
    if parent_pid != actual_parent {
        return Err(ServiceError::authentication("当前进程的父进程与请求不一致"));
    }
    let session_id = process_session_id(pid)?;
    let sid = process_sid(pid)?;
    Ok(ClientIdentity {
        pid,
        parent_pid,
        session_id,
        sid,
    })
}

pub(crate) fn process_parent_pid(pid: u32) -> ServiceResult<u32> {
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(ServiceError::from)?;
    let _snapshot_guard = HandleGuard(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let first = unsafe { Process32FirstW(snapshot, &mut entry) };
    if first.is_err() {
        return Err(ServiceError::from(first.unwrap_err()));
    }
    loop {
        if entry.th32ProcessID == pid {
            return Ok(entry.th32ParentProcessID);
        }
        let next = unsafe { Process32NextW(snapshot, &mut entry) };
        if next.is_err() {
            break;
        }
    }
    Err(ServiceError::authentication(format!(
        "无法读取进程 {pid} 的父进程"
    )))
}

pub(crate) fn process_session_id(pid: u32) -> ServiceResult<u32> {
    let mut session = 0u32;
    unsafe { ProcessIdToSessionId(pid, &mut session) }.map_err(ServiceError::from)?;
    Ok(session)
}

pub(crate) fn process_sid(pid: u32) -> ServiceResult<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(ServiceError::from)?;
    let _process_guard = HandleGuard(process);
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(ServiceError::from)?;
    let _token_guard = HandleGuard(token);
    token_sid(token)
}

fn token_sid(token: HANDLE) -> ServiceResult<String> {
    let mut needed = 0u32;
    let first = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if first.is_ok() || needed == 0 {
        return Err(ServiceError::internal("读取令牌用户 SID 大小失败"));
    }
    let mut bytes = vec![0u8; needed as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(bytes.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .map_err(ServiceError::from)?;
    let user = unsafe { &*(bytes.as_ptr() as *const TOKEN_USER) };
    let mut sid_string = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_string) }
        .map_err(ServiceError::from)?;
    if sid_string.is_null() {
        return Err(ServiceError::internal("令牌用户 SID 为空"));
    }
    let value = unsafe { sid_string.to_string() }
        .map_err(|error| ServiceError::internal(error.to_string()))?;
    unsafe {
        LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            sid_string.0 as *mut std::ffi::c_void,
        )));
    }
    Ok(value)
}

pub(crate) fn validate_client(
    pipe: HANDLE,
    expected_pid: u32,
    expected_parent_pid: u32,
    expected_session: u32,
    expected_sid: &str,
) -> ServiceResult<()> {
    let mut pipe_pid = 0u32;
    unsafe { windows::Win32::System::Pipes::GetNamedPipeClientProcessId(pipe, &mut pipe_pid) }
        .map_err(ServiceError::from)?;
    if pipe_pid == 0 || pipe_pid != expected_pid {
        return Err(ServiceError::authentication("命名管道客户端进程身份不匹配"));
    }
    let mut pipe_session = 0u32;
    unsafe { windows::Win32::System::Pipes::GetNamedPipeClientSessionId(pipe, &mut pipe_session) }
        .map_err(ServiceError::from)?;
    if pipe_session != expected_session || process_session_id(pipe_pid)? != expected_session {
        return Err(ServiceError::authentication("命名管道客户端会话不匹配"));
    }
    if process_parent_pid(pipe_pid)? != expected_parent_pid {
        return Err(ServiceError::authentication("命名管道客户端父进程不匹配"));
    }
    let sid = process_sid(pipe_pid)?;
    if sid != expected_sid {
        return Err(ServiceError::authentication("命名管道客户端 SID 不匹配"));
    }
    Ok(())
}

pub(crate) struct HandleGuard(pub HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}
