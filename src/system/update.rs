//! GitHub Release 检查与安装包完整性校验。

use std::fs::{self, File};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Debug, Clone)]
pub struct InstallerAsset {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub enum CheckResult {
    UpToDate,
    Available {
        tag: String,
        installer: Option<InstallerAsset>,
    },
}

pub const LATEST_RELEASE_URL: &str = "https://github.com/NGLSL/Kite/releases/latest";
pub const REPOSITORY_URL: &str = "https://github.com/NGLSL/Kite";
const RELEASE_API_URL: &str = "https://api.github.com/repos/NGLSL/Kite/releases/latest";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

pub fn check_latest(current: &str) -> Result<CheckResult, String> {
    let output = Command::new("curl.exe")
        .creation_flags(CREATE_NO_WINDOW.0)
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            "User-Agent: Kite",
            "--url",
            RELEASE_API_URL,
        ])
        .output()
        .map_err(|e| format!("无法启动版本检查: {e}"))?;
    if !output.status.success() {
        return Err(format!("GitHub 版本检查失败 ({})", output.status));
    }
    parse_release_json(&output.stdout, current)
}

fn parse_release_json(body: &[u8], current: &str) -> Result<CheckResult, String> {
    let release: GithubRelease =
        serde_json::from_slice(body).map_err(|e| format!("GitHub 版本信息无法解析: {e}"))?;
    let latest = version_parts(&release.tag_name).ok_or("GitHub 版本号无效")?;
    let installed = version_parts(current).ok_or("当前版本号无效")?;
    if latest <= installed {
        return Ok(CheckResult::UpToDate);
    }

    let installer = release.assets.into_iter().find_map(|asset| {
        if asset.name != "kite-setup.exe"
            || asset.size == 0
            || !asset.browser_download_url.starts_with(
                "https://github.com/NGLSL/Kite/releases/download/",
            )
            || !asset.browser_download_url.ends_with("/kite-setup.exe")
        {
            return None;
        }
        let digest = asset.digest?.strip_prefix("sha256:")?.to_ascii_lowercase();
        if digest.len() != 64 || !digest.bytes().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        Some(InstallerAsset {
            url: asset.browser_download_url,
            size: asset.size,
            sha256: digest,
        })
    });
    Ok(CheckResult::Available {
        tag: release.tag_name,
        installer,
    })
}

fn version_parts(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.trim_start_matches(['v', 'V']).split('.');
    let parsed = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(parsed)
}

pub fn download_verified(asset: &InstallerAsset) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("kite-updates");
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建更新目录: {e}"))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("系统时间无效: {e}"))?
        .as_nanos();
    let name = format!("kite-update-{}-{stamp}", std::process::id());
    let pending = dir.join(format!("{name}.part"));
    let ready = dir.join(format!("{name}.exe"));

    let result = (|| {
        let status = Command::new("curl.exe")
            .creation_flags(CREATE_NO_WINDOW.0)
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--connect-timeout",
                "10",
                "--max-time",
                "120",
                "--output",
            ])
            .arg(&pending)
            .arg("--url")
            .arg(&asset.url)
            .status()
            .map_err(|e| format!("无法启动更新下载: {e}"))?;
        if !status.success() {
            return Err(format!("安装包下载失败 ({status})"));
        }
        verify_installer(&pending, asset)?;
        fs::rename(&pending, &ready).map_err(|e| format!("无法准备安装包: {e}"))?;
        Ok(ready)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
    }
    result
}

fn verify_installer(path: &Path, asset: &InstallerAsset) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("安装包无法读取: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("安装包大小无法读取: {e}"))?
        .len();
    if size != asset.size {
        return Err(format!("安装包大小不符: 预期 {}，实际 {size}", asset.size));
    }
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk).map_err(|e| format!("安装包校验失败: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    if format!("{:x}", hasher.finalize()) != asset.sha256 {
        return Err("安装包 SHA-256 校验失败".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn release_json(digest: &str) -> Vec<u8> {
        format!(
            r#"{{"tag_name":"v0.2.5","assets":[{{"name":"kite-setup.exe","browser_download_url":"https://github.com/NGLSL/Kite/releases/download/v0.2.5/kite-setup.exe","size":7,"digest":"{digest}"}}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn accepts_only_verified_new_installer_asset() {
        let digest = format!("sha256:{:x}", Sha256::digest(b"payload"));
        let result = parse_release_json(&release_json(&digest), "0.2.4").unwrap();
        let CheckResult::Available { tag, installer } = result else {
            panic!("new release expected")
        };
        assert_eq!(tag, "v0.2.5");
        let installer = installer.expect("verified installer asset");
        assert_eq!(installer.size, 7);
        assert_eq!(installer.sha256, digest.trim_start_matches("sha256:"));
    }

    #[test]
    fn missing_digest_requires_manual_download() {
        let result = parse_release_json(&release_json(""), "0.2.4").unwrap();
        assert!(matches!(result, CheckResult::Available { installer: None, .. }));
    }

    #[test]
    fn current_release_does_not_offer_installer() {
        let result = parse_release_json(&release_json(""), "0.2.5").unwrap();
        assert!(matches!(result, CheckResult::UpToDate));
    }

    #[test]
    fn verifies_payload_and_rejects_corruption() {
        let path = std::env::temp_dir().join(format!("kite-update-test-{}", std::process::id()));
        let asset = InstallerAsset {
            url: "https://github.com/NGLSL/Kite/releases/download/v0.2.5/kite-setup.exe".into(),
            size: 7,
            sha256: format!("{:x}", Sha256::digest(b"payload")),
        };
        std::fs::write(&path, b"payload").unwrap();
        assert!(verify_installer(&path, &asset).is_ok());
        std::fs::write(&path, b"corrupt").unwrap();
        let result = verify_installer(&path, &asset);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_err());
    }

    #[test]
    #[ignore = "需要访问 GitHub 并下载真实安装包，手动运行"]
    fn live_release_download() {
        let CheckResult::Available { installer, .. } = check_latest("0.2.3").unwrap() else {
            panic!("expected a release newer than 0.2.3")
        };
        let path = download_verified(&installer.expect("release asset with SHA-256")).unwrap();
        assert!(path.is_file());
        std::fs::remove_file(path).unwrap();
    }
}
