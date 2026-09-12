//! Everything 文件搜索：调用已安装的 Everything，不自建 NTFS 索引。
//! Everything 未安装或未运行时静默返回空，不影响应用搜索。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 文件命中。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EverythingHit {
    pub path: String,
    pub is_folder: bool,
    pub name: String,
}

fn everything_exe() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        r"D:\Program Files\Everything\Everything.exe",
        r"C:\Program Files\Everything\Everything.exe",
        r"C:\Program Files (x86)\Everything\Everything.exe",
    ];
    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .or_else(|| {
            // PATH
            let out = Command::new("where")
                .arg("Everything.exe")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .ok()?;
            let line = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().to_string();
            if line.is_empty() {
                None
            } else {
                Some(PathBuf::from(line))
            }
        })
}

/// 查询 Everything；`max` 为返回上限。
pub fn search_files(query: &str, max: usize) -> Vec<EverythingHit> {
    let q = query.trim();
    if q.is_empty() || max == 0 {
        return Vec::new();
    }
    let Some(exe) = everything_exe() else {
        return Vec::new();
    };

    let out = std::env::temp_dir().join(format!("kite-everything-{}.csv", std::process::id()));
    let _ = std::fs::remove_file(&out);

    // Everything 已在后台时会把命令交给现有实例；CREATE_NO_WINDOW 避免闪窗。
    let mut cmd = Command::new(&exe);
    cmd.arg("-s")
        .arg(q)
        .arg("-export-csv")
        .arg(&out)
        .arg("-n")
        .arg(max.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // 给导出一点时间；失败则空
    if let Ok(mut child) = cmd.spawn() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
        loop {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    parse_csv(&out, max)
}

fn parse_csv(path: &Path, max: usize) -> Vec<EverythingHit> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(path);
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 && line.to_lowercase().contains("path") {
            continue;
        }
        let path = unquote_csv(line);
        if path.is_empty() || !Path::new(&path).exists() {
            continue;
        }
        let is_folder = Path::new(&path).is_dir();
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        hits.push(EverythingHit {
            path,
            is_folder,
            name,
        });
        if hits.len() >= max {
            break;
        }
    }
    hits
}

fn unquote_csv(line: &str) -> String {
    let s = line.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].replace("\"\"", "\"")
    } else {
        s.to_string()
    }
}
