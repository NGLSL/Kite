//! 探测本机已安装的常见浏览器（只查固定路径，不做全盘扫描）。

use std::path::{Path, PathBuf};

/// 一个可用浏览器。
#[derive(Debug, Clone)]
pub struct Browser {
    /// 稳定短 id，用于偏好记忆（chrome / edge / …）。
    pub id: String,
    pub name: String,
    pub exe: PathBuf,
}

/// 候选浏览器：id、显示名、常见安装路径（含 Program Files / x86 / LocalAppData）。
fn candidates() -> Vec<(String, String, Vec<String>)> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let pf86 = std::env::var("ProgramFiles(x86)")
        .unwrap_or_else(|_| r"C:\Program Files (x86)".into());

    vec![
        (
            "chrome".into(),
            "Google Chrome".into(),
            vec![
                format!(r"{pf}\Google\Chrome\Application\chrome.exe"),
                format!(r"{pf86}\Google\Chrome\Application\chrome.exe"),
                format!(r"{local}\Google\Chrome\Application\chrome.exe"),
            ],
        ),
        (
            "edge".into(),
            "Microsoft Edge".into(),
            vec![
                format!(r"{pf86}\Microsoft\Edge\Application\msedge.exe"),
                format!(r"{pf}\Microsoft\Edge\Application\msedge.exe"),
            ],
        ),
        (
            "firefox".into(),
            "Mozilla Firefox".into(),
            vec![
                format!(r"{pf}\Mozilla Firefox\firefox.exe"),
                format!(r"{pf86}\Mozilla Firefox\firefox.exe"),
            ],
        ),
        (
            "brave".into(),
            "Brave".into(),
            vec![
                format!(r"{pf}\BraveSoftware\Brave-Browser\Application\brave.exe"),
                format!(r"{local}\BraveSoftware\Brave-Browser\Application\brave.exe"),
            ],
        ),
        (
            "vivaldi".into(),
            "Vivaldi".into(),
            vec![
                format!(r"{pf}\Vivaldi\Application\vivaldi.exe"),
                format!(r"{local}\Vivaldi\Application\vivaldi.exe"),
            ],
        ),
        (
            "opera".into(),
            "Opera".into(),
            vec![
                format!(r"{local}\Programs\Opera\opera.exe"),
                format!(r"{pf}\Opera\opera.exe"),
            ],
        ),
        (
            "arc".into(),
            "Arc".into(),
            vec![format!(r"{local}\Programs\Arc\Arc.exe")],
        ),
        (
            "360se".into(),
            "360 安全浏览器".into(),
            vec![
                format!(r"{pf}\360\360se6\Application\360se.exe"),
                format!(r"{pf86}\360\360se6\Application\360se.exe"),
            ],
        ),
        (
            "qqbrowser".into(),
            "QQ 浏览器".into(),
            vec![
                format!(r"{pf}\Tencent\QQBrowser\QQBrowser.exe"),
                format!(r"{pf86}\Tencent\QQBrowser\QQBrowser.exe"),
            ],
        ),
    ]
}

/// 发现已安装浏览器；结果稳定按 id 排序。进程内缓存，避免每次按键扫盘。
pub fn discover_installed() -> Vec<Browser> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<Browser>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut out = Vec::new();
            for (id, name, paths) in candidates() {
                if let Some(exe) = paths.into_iter().map(PathBuf::from).find(|p| p.is_file()) {
                    out.push(Browser { id, name, exe });
                }
            }
            out.sort_by(|a, b| a.id.cmp(&b.id));
            out
        })
        .clone()
}

/// 偏好浏览器排前，其余保持原序；preferred 未安装则忽略。
pub fn sort_preferred(browsers: Vec<Browser>, preferred: Option<&str>) -> Vec<Browser> {
    let Some(pref) = preferred.filter(|s| !s.is_empty()) else {
        return browsers;
    };
    let mut head = Vec::new();
    let mut rest = Vec::new();
    for b in browsers {
        if b.id == pref {
            head.push(b);
        } else {
            rest.push(b);
        }
    }
    head.extend(rest);
    head
}

/// 用指定浏览器打开 URL。
pub fn open_url(browser_exe: &Path, url: &str) -> Result<(), String> {
    if !browser_exe.is_file() {
        return Err(format!("browser not found: {}", browser_exe.display()));
    }
    std::process::Command::new(browser_exe)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn browser failed: {e}"))?;
    Ok(())
}

/// 系统默认浏览器打开（ShellExecute）。
pub fn open_with_default(url: &str) -> Result<(), String> {
    crate::app::uwp::launch_shell_path(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser(id: &str) -> Browser {
        Browser {
            id: id.into(),
            name: id.into(),
            exe: PathBuf::from(format!(r"C:\fake\{id}.exe")),
        }
    }

    #[test]
    fn preferred_first_others_kept() {
        let list = vec![browser("chrome"), browser("edge"), browser("firefox")];
        let sorted = sort_preferred(list, Some("edge"));
        assert_eq!(sorted[0].id, "edge");
        assert_eq!(sorted.len(), 3);
        assert!(sorted.iter().any(|b| b.id == "chrome"));
        assert!(sorted.iter().any(|b| b.id == "firefox"));
    }

    #[test]
    fn missing_preferred_keeps_order() {
        let list = vec![browser("chrome"), browser("edge")];
        let sorted = sort_preferred(list, Some("safari"));
        assert_eq!(sorted[0].id, "chrome");
        assert_eq!(sorted[1].id, "edge");
    }

    #[test]
    fn no_preferred_keeps_order() {
        let list = vec![browser("firefox"), browser("chrome")];
        let sorted = sort_preferred(list, None);
        assert_eq!(sorted[0].id, "firefox");
    }
}
