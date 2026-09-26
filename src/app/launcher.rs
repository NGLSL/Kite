use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;

use crate::model::AppItem;
use crate::system::env::expand_env;

use super::uwp;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CliTerminal {
    #[default]
    Auto,
    WindowsTerminal,
    PowerShell,
    Cmd,
}

impl CliTerminal {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "windows_terminal" => Self::WindowsTerminal,
            "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Auto,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::WindowsTerminal => "windows_terminal",
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
        }
    }
}

pub fn launch_with_terminal(item: &AppItem, terminal: CliTerminal) -> Result<(), String> {
    let path = Path::new(&item.target);
    if item.source == "commands" && item.args.is_none() && is_batch_script(path) {
        if !path.is_file() {
            return Err(format!("target not found: {}", item.target));
        }
        return launch_cli_script(path, terminal);
    }
    launch(item)
}

#[derive(Default)]
struct AvailableTerminals {
    wt: Option<PathBuf>,
    powershell: Option<PathBuf>,
    cmd: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalKind {
    WindowsTerminal,
    PowerShell,
    Cmd,
}

impl AvailableTerminals {
    fn detect() -> Self {
        let windows = std::env::var_os("SystemRoot").map(PathBuf::from);
        let wt = find_on_path("wt.exe").or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join(r"Microsoft\WindowsApps\wt.exe"))
                .filter(|p| p.is_file())
        });
        let powershell = find_on_path("pwsh.exe")
            .or_else(|| find_on_path("powershell.exe"))
            .or_else(|| {
                windows
                    .as_ref()
                    .map(|p| p.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
                    .filter(|p| p.is_file())
            });
        let cmd = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| {
                windows
                    .as_ref()
                    .map(|p| p.join(r"System32\cmd.exe"))
                    .filter(|p| p.is_file())
            });
        Self {
            wt,
            powershell,
            cmd,
        }
    }

    fn choose(&self, preferred: CliTerminal) -> Result<TerminalKind, String> {
        match preferred {
            CliTerminal::Auto => {
                if self.wt.is_some() && self.powershell.is_some() {
                    Ok(TerminalKind::WindowsTerminal)
                } else if self.powershell.is_some() {
                    Ok(TerminalKind::PowerShell)
                } else if self.cmd.is_some() {
                    Ok(TerminalKind::Cmd)
                } else {
                    Err("未找到可用终端".into())
                }
            }
            CliTerminal::WindowsTerminal if self.wt.is_some() && self.powershell.is_some() => {
                Ok(TerminalKind::WindowsTerminal)
            }
            CliTerminal::PowerShell if self.powershell.is_some() => Ok(TerminalKind::PowerShell),
            CliTerminal::Cmd if self.cmd.is_some() => Ok(TerminalKind::Cmd),
            _ => Err("所选终端不可用，请在设置中更换启动方式".into()),
        }
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn is_batch_script(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case(OsStr::new("cmd"))
            || extension.eq_ignore_ascii_case(OsStr::new("bat"))
    })
}

fn powershell_encoded_command(path: &Path) -> String {
    let literal = path.to_string_lossy().replace('\'', "''");
    let script = format!("& '{literal}'");
    let utf16le: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16le)
}

fn launch_cli_script(path: &Path, preferred: CliTerminal) -> Result<(), String> {
    let available = AvailableTerminals::detect();
    let kind = available.choose(preferred)?;
    // PATH shims are installed beside Node/package-manager binaries. Running from that directory
    // makes interactive CLIs act on the install folder, so use the user's home as their cwd.
    let cwd = resolve_working_dir(None).ok_or("无法确定终端工作目录")?;
    let mut command = match kind {
        TerminalKind::WindowsTerminal => {
            let mut cmd = Command::new(available.wt.as_ref().unwrap());
            cmd.arg("new-tab")
                .arg("-d")
                .arg(&cwd)
                .arg(available.powershell.as_ref().unwrap())
                .arg("-NoLogo")
                .arg("-NoExit")
                .arg("-EncodedCommand")
                .arg(powershell_encoded_command(path));
            cmd
        }
        TerminalKind::PowerShell => {
            let mut cmd = Command::new(available.powershell.as_ref().unwrap());
            cmd.arg("-NoLogo")
                .arg("-NoExit")
                .arg("-EncodedCommand")
                .arg(powershell_encoded_command(path));
            cmd
        }
        TerminalKind::Cmd => {
            let mut cmd = Command::new(available.cmd.as_ref().unwrap());
            // Expand the indexed path once inside quotes. /s keeps the inner quote pair;
            // do not use `call`, which expands paired %NAME% in a filename a second time.
            cmd.arg("/d")
                .arg("/v:off")
                .arg("/s")
                .arg("/k")
                .env("KITE_CLI_TARGET", path);
            cmd.raw_arg("\"\"%KITE_CLI_TARGET%\"\"");
            cmd
        }
    };
    command.current_dir(cwd);
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动终端失败: {e}"))
}

/// 启动工作目录：条目自带的起始位置（展开 %VAR%）优先，否则用户主目录。
/// 终端类应用（WT / PowerShell / cmd）会把 cwd 当 shell 初始路径展示给用户，
/// 兜底不能落到 exe 目录或 Kite 安装目录。
fn resolve_working_dir(working_dir: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = working_dir {
        let expanded = expand_env(dir);
        let p = PathBuf::from(expanded);
        if p.is_dir() {
            return Some(p);
        }
    }
    dirs::home_dir().filter(|h| h.is_dir())
}

/// 启动索引中的应用。禁止把用户输入拼进 shell 字符串。
pub fn launch(item: &AppItem) -> Result<(), String> {
    let target = &item.target;
    if target.is_empty() {
        return Err("empty target".into());
    }

    // Store / shell / 系统设置 URI
    if target.starts_with("shell:")
        || target.starts_with("ms-settings:")
        || target.starts_with("ms-clock:")
        || target.starts_with("ms-contact-support:")
    {
        return uwp::launch_shell_path(target);
    }

    // 内置动作由 commands 层处理；此处不认 kite:
    if target.starts_with("kite:") {
        return Err("builtin action should be handled by launch_app".into());
    }

    // 文件/文件夹（Everything 结果）
    let path = Path::new(target);
    if path.is_dir() {
        return opener_open(target);
    }
    if !path.exists() {
        return Err(format!("target not found: {target}"));
    }

    // 文件用系统关联打开
    if !is_executable(path) {
        return opener_open(target);
    }

    let mut cmd = Command::new(target);
    if let Some(args) = &item.args {
        // .lnk 参数已是 Windows 命令行片段，保留其引号和转义。
        cmd.raw_arg(args);
    }
    // working_dir 无效时回落用户主目录，而不是继承 Kite 自己的 cwd
    let working_dir = resolve_working_dir(if item.source == "commands" {
        // Also override older cached command rows that recorded the CLI installation directory.
        None
    } else {
        item.working_dir.as_deref()
    });
    if let Some(dir) = &working_dir {
        cmd.current_dir(dir);
    }

    // 交互式控制台需要 Windows 分配的标准句柄；stdin=NUL 会让 PowerShell 读到 EOF 后退出。
    match cmd.spawn() {
        Ok(_) => Ok(()),
        // requireAdministrator 清单的程序：CreateProcess 无法触发 UAC（os error 740），
        // 退回 ShellExecute("runas") 弹授权框，与资源管理器双击行为一致
        Err(e) if elevation_required(&e) => super::uwp::launch_runas(
            target,
            item.args.as_deref().unwrap_or(""),
            working_dir.as_deref(),
        ),
        Err(e) => Err(format!("spawn failed: {e}")),
    }
}

/// os error 740 = ERROR_ELEVATION_REQUIRED：目标 exe 的清单要求管理员权限。
fn elevation_required(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(740)
}

fn is_executable(p: &Path) -> bool {
    p.extension()
        .map(|e| e.to_string_lossy().eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn opener_open(path: &str) -> Result<(), String> {
    // 使用 tauri-plugin-opener 会更稳；此处用 ShellExecute 同源能力
    super::uwp::launch_shell_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_script_starts_in_user_home() {
        let fixture = std::env::temp_dir().join(format!(
            "kite-cli-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&fixture).unwrap();
        let script = fixture.join("cli & %TEMP% 100%.cmd");
        let output = fixture.join("cwd.txt");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\necho %CD%>\"{}\"\r\nexit\r\n",
                output.display()
            ),
        )
        .unwrap();
        let item = AppItem::scanned(
            "cli-script-test".into(),
            "cli script".into(),
            script.to_string_lossy().into_owned(),
            None,
            None,
            "commands",
        );
        launch_with_terminal(&item, CliTerminal::Cmd).expect("launch command script");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let cwd = loop {
            if let Ok(value) = std::fs::read_to_string(&output) {
                break value;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "script did not write cwd"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(
            cwd.trim().to_lowercase(),
            dirs::home_dir().unwrap().to_string_lossy().to_lowercase()
        );
        let _ = std::fs::remove_dir_all(&fixture);
    }

    #[test]
    fn terminal_selection_prefers_terminal_then_powershell_then_cmd() {
        let all = AvailableTerminals {
            wt: Some("wt.exe".into()),
            powershell: Some("powershell.exe".into()),
            cmd: Some("cmd.exe".into()),
        };
        assert_eq!(
            all.choose(CliTerminal::Auto).unwrap(),
            TerminalKind::WindowsTerminal
        );
        let without_wt = AvailableTerminals { wt: None, ..all };
        assert_eq!(
            without_wt.choose(CliTerminal::Auto).unwrap(),
            TerminalKind::PowerShell
        );
        let only_cmd = AvailableTerminals {
            powershell: None,
            ..without_wt
        };
        assert_eq!(
            only_cmd.choose(CliTerminal::Auto).unwrap(),
            TerminalKind::Cmd
        );
        assert!(only_cmd.choose(CliTerminal::WindowsTerminal).is_err());
        assert_eq!(CliTerminal::from_setting("unknown"), CliTerminal::Auto);
    }

    #[test]
    fn powershell_encoded_command_runs_quoted_batch_path() {
        let fixture = std::env::temp_dir().join(format!(
            "kite-cli-powershell-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&fixture).unwrap();
        let script = fixture.join("cli & ' 100%.cmd");
        let output = fixture.join("ran.txt");
        std::fs::write(
            &script,
            format!("@echo off\r\necho reached>\"{}\"\r\n", output.display()),
        )
        .unwrap();
        let powershell = AvailableTerminals::detect()
            .powershell
            .expect("Windows test machine needs PowerShell");
        let status = Command::new(powershell)
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-EncodedCommand")
            .arg(powershell_encoded_command(&script))
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(&output).unwrap().trim(), "reached");
        let _ = std::fs::remove_dir_all(&fixture);
    }

    #[test]
    fn explicit_valid_dir_wins() {
        let tmp = std::env::temp_dir();
        let got = resolve_working_dir(Some(tmp.to_str().unwrap()));
        assert_eq!(got, Some(tmp));
    }

    #[test]
    fn invalid_or_missing_dir_falls_back_to_home() {
        let home = dirs::home_dir().expect("Windows 测试机必有主目录");
        assert_eq!(resolve_working_dir(None), Some(home.clone()));
        assert_eq!(resolve_working_dir(Some(r"Q:\no\such\dir")), Some(home));
    }

    #[test]
    fn env_vars_in_working_dir_are_expanded() {
        // lnk 起始位置常见 %HOMEDRIVE%%HOMEPATH% 这类未展开引用
        assert_eq!(
            resolve_working_dir(Some("%TEMP%")),
            Some(std::env::temp_dir())
        );
    }

    #[test]
    fn expand_env_keeps_unknown_var() {
        assert_eq!(
            expand_env(r"%KITE_NO_SUCH_VAR_9%\x"),
            r"%KITE_NO_SUCH_VAR_9%\x"
        );
    }

    #[test]
    fn os_error_740_is_elevation_required() {
        assert!(elevation_required(&std::io::Error::from_raw_os_error(740)));
        assert!(!elevation_required(&std::io::Error::from_raw_os_error(2)));
    }

    /// 回归：无 working_dir（或指向无效路径）时，子进程 cwd 必须是用户主目录，
    /// 而不是 Kite 安装目录（用户可见症状：终端打开后停在 D:\...\Kite）。
    #[test]
    fn launch_without_working_dir_starts_in_home() {
        let out = std::env::temp_dir().join(format!("kite-cwd-cmd-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let cmd_path =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        let item = AppItem::scanned(
            "cwd-test".into(),
            "cmd".into(),
            cmd_path,
            Some(format!("/c cd > \"{}\"", out.display())),
            None,
            "test",
        );
        launch(&item).expect("spawn cmd");

        let home = dirs::home_dir().unwrap();
        let home_str = home.to_string_lossy().to_lowercase();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(s) = std::fs::read_to_string(&out) {
                let cwd = s.trim().to_lowercase();
                assert_eq!(cwd, home_str, "cmd 初始 cwd 应为用户主目录");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "cmd 未在期限内写出 cwd"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn command_source_exe_ignores_cached_install_directory() {
        let out = std::env::temp_dir().join(format!(
            "kite-cli-exe-cwd-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out);
        let cmd_path =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        let item = AppItem::scanned(
            "cli-cwd-test".into(),
            "cmd".into(),
            cmd_path,
            Some(format!("/c cd > \"{}\"", out.display())),
            Some(std::env::temp_dir().to_string_lossy().into_owned()),
            "commands",
        );
        launch(&item).expect("spawn command source exe");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let cwd = loop {
            if let Ok(value) = std::fs::read_to_string(&out) {
                break value;
            }
            assert!(std::time::Instant::now() < deadline, "CLI did not write cwd");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(
            cwd.trim().to_lowercase(),
            dirs::home_dir().unwrap().to_string_lossy().to_lowercase()
        );
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn launch_preserves_quoted_shortcut_arguments() {
        let out = std::env::temp_dir().join(format!("kite quoted args {}.txt", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let cmd_path =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        let item = AppItem::scanned(
            "quoted-args-test".into(),
            "cmd".into(),
            cmd_path,
            Some(format!("/c echo quoted > \"{}\"", out.display())),
            None,
            "test",
        );
        launch(&item).expect("spawn cmd with quoted arguments");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Ok(contents) = std::fs::read_to_string(&out) {
                assert_eq!(contents.trim(), "quoted");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "quoted path was not passed intact to cmd"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = std::fs::remove_file(out);
    }
}
