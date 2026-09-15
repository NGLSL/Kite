//! Keep the interactive Kite process at the user's normal integrity level.
//!
//! The NSIS installer must run elevated so it can write `Program Files` and
//! HKLM. Windows can inherit that elevated token when the finish page starts
//! Kite, though. That makes low-integrity/medium-integrity global hotkey tools
//! (for example screenshot utilities) unable to deliver their keys to Kite.
//! If Kite starts elevated, recreate it from Explorer's user token and let the
//! elevated bootstrap process exit before the UI and hotkey hooks are created.

use std::mem::size_of;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TokenElevation, TokenPrimary,
    TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    CreateProcessWithTokenW, GetCurrentProcess, OpenProcess, OpenProcessToken,
    CREATE_UNICODE_ENVIRONMENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};

/// Explicit marker passed to the child created by the elevated bootstrap
/// process. A command-line marker is used because CreateProcessWithTokenW
/// with LOGON_WITH_PROFILE may rebuild the child environment instead of
/// inheriting variables set by the elevated parent.
const DEELEVATION_ATTEMPT_ARG: &str = "--kite-after-deelevation";

/// Returns `Ok(true)` when this process was elevated and a normal child was
/// started. The caller must exit in that case. `Ok(false)` means normal
/// execution may continue.
pub fn relaunch_if_elevated() -> Result<bool, String> {
    let current_elevated = is_elevated()?;
    let has_marker = has_deelevation_marker();
    crate::log::info(&format!(
        "elevation status current={} deelevation_arg={has_marker}",
        current_elevated
    ));

    if !current_elevated {
        return Ok(false);
    }

    if has_marker {
        crate::log::info("de-elevation child is still elevated; skip another relaunch attempt");
        return Ok(false);
    }

    crate::log::info("elevated bootstrap: relaunching with Explorer token");
    // `false` means Explorer itself is elevated. In that configuration there
    // is no lower-integrity token to use, so continue in this process instead
    // of creating an unbounded chain of equally elevated children.
    relaunch_from_explorer_token()
}

fn is_elevated() -> Result<bool, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| format!("OpenProcessToken(current): {e}"))?;
        let result = token_is_elevated(token);
        let _ = CloseHandle(token);
        result
    }
}

fn has_deelevation_marker() -> bool {
    if std::env::args_os().any(|arg| arg.to_string_lossy() == DEELEVATION_ATTEMPT_ARG) {
        return true;
    }

    // Keep a second check against the Win32 command line. It makes the guard
    // effective even if a launcher has unusual argv handling, and records the
    // same decision the process receives from CreateProcessWithTokenW.
    unsafe {
        windows::Win32::System::Environment::GetCommandLineW()
            .to_string()
            .ok()
            .is_some_and(|line| command_line_has_marker(&line))
    }
}

fn command_line_has_marker(line: &str) -> bool {
    line.split_whitespace()
        .any(|arg| arg.trim_matches('"') == DEELEVATION_ATTEMPT_ARG)
}

fn token_is_elevated(token: HANDLE) -> Result<bool, String> {
    unsafe {
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .map_err(|e| format!("GetTokenInformation(TokenElevation): {e}"))?;
        Ok(elevation.TokenIsElevated != 0)
    }
}

fn relaunch_from_explorer_token() -> Result<bool, String> {
    unsafe {
        let shell = GetShellWindow();
        if shell.0.is_null() {
            return Err("GetShellWindow returned no Explorer shell".to_string());
        }

        let mut explorer_pid = 0u32;
        if GetWindowThreadProcessId(shell, Some(&mut explorer_pid)) == 0 || explorer_pid == 0 {
            return Err("could not resolve the Explorer process".to_string());
        }

        let explorer = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, explorer_pid)
            .map_err(|e| format!("OpenProcess(explorer): {e}"))?;
        let mut explorer_token = HANDLE::default();
        let result = OpenProcessToken(
            explorer,
            TOKEN_ACCESS_MASK(
                TOKEN_DUPLICATE.0
                    | TOKEN_ASSIGN_PRIMARY.0
                    | TOKEN_QUERY.0
                    | TOKEN_ADJUST_DEFAULT.0
                    | TOKEN_ADJUST_SESSIONID.0,
            ),
            &mut explorer_token,
        );
        let _ = CloseHandle(explorer);
        result.map_err(|e| format!("OpenProcessToken(explorer): {e}"))?;

        let explorer_elevated = match token_is_elevated(explorer_token) {
            Ok(value) => value,
            Err(error) => {
                let _ = CloseHandle(explorer_token);
                return Err(error);
            }
        };
        crate::log::info(&format!(
            "elevated bootstrap: Explorer token pid={explorer_pid} elevated={explorer_elevated}"
        ));
        if explorer_elevated {
            let _ = CloseHandle(explorer_token);
            crate::log::info(
                "Explorer token is already elevated; continue without de-elevation relaunch",
            );
            return Ok(false);
        }

        let mut primary_token = HANDLE::default();
        let result = DuplicateTokenEx(
            explorer_token,
            TOKEN_ACCESS_MASK(
                TOKEN_DUPLICATE.0
                    | TOKEN_ASSIGN_PRIMARY.0
                    | TOKEN_QUERY.0
                    | TOKEN_ADJUST_DEFAULT.0
                    | TOKEN_ADJUST_SESSIONID.0,
            ),
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary_token,
        );
        let _ = CloseHandle(explorer_token);
        result.map_err(|e| format!("DuplicateTokenEx(explorer): {e}"))?;

        let executable = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let executable = executable
            .to_str()
            .ok_or_else(|| "current executable path is not valid UTF-8".to_string())?;
        let executable_wide: Vec<u16> = executable
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut command_line: Vec<u16> = format!("\"{executable}\" {DEELEVATION_ATTEMPT_ARG}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut process_info = PROCESS_INFORMATION::default();

        let result = CreateProcessWithTokenW(
            primary_token,
            LOGON_WITH_PROFILE,
            PCWSTR(executable_wide.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &mut startup,
            &mut process_info,
        );
        let _ = CloseHandle(primary_token);
        result.map_err(|e| format!("CreateProcessWithTokenW: {e}"))?;

        let _ = CloseHandle(process_info.hThread);
        let _ = CloseHandle(process_info.hProcess);
        crate::log::info(&format!(
            "de-elevation child launched pid={} with marker",
            process_info.dwProcessId
        ));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_marker_is_detected() {
        assert!(command_line_has_marker(
            r#""C:\Program Files\Kite\kite.exe" --kite-after-deelevation"#
        ));
        assert!(!command_line_has_marker(
            r#""C:\Program Files\Kite\kite.exe" --other"#
        ));
    }
}
