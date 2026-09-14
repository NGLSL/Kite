use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::AppItem;
use crate::system::env::expand_env;

use super::uwp;

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
    let working_dir = resolve_working_dir(item.working_dir.as_deref());
    if let Some(dir) = &working_dir {
        cmd.current_dir(dir);
    }

    // 交互式控制台需要 Windows 分配的标准句柄；stdin=NUL 会让 PowerShell 读到 EOF 后退出。
    match cmd.spawn() {
        Ok(_) => Ok(()),
        // requireAdministrator 清单的程序：CreateProcess 无法触发 UAC（os error 740），
        // 退回 ShellExecute("runas") 弹授权框，与资源管理器双击行为一致
        Err(e) if elevation_required(&e) => {
            super::uwp::launch_runas(target, item.args.as_deref().unwrap_or(""), working_dir.as_deref())
        }
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
        assert_eq!(resolve_working_dir(Some("%TEMP%")), Some(std::env::temp_dir()));
    }

    #[test]
    fn expand_env_keeps_unknown_var() {
        assert_eq!(expand_env(r"%KITE_NO_SUCH_VAR_9%\x"), r"%KITE_NO_SUCH_VAR_9%\x");
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
        let cmd_path = std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
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
            assert!(std::time::Instant::now() < deadline, "cmd 未在期限内写出 cwd");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn launch_preserves_quoted_shortcut_arguments() {
        let out = std::env::temp_dir().join(format!(
            "kite quoted args {}.txt",
            std::process::id()
        ));
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
