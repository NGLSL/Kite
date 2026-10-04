//! JSON/二进制侧文件的原子覆盖写。
//!
//! Windows 上「先删再 rename」存在空窗：进程被杀或读者撞上时可能读到空文件。
//! 索引快照已用 `ReplaceFileW` / `MoveFileExW`；本模块把同一路径收成公共入口，
//! LNK/UWP/Meta 等缓存与 snapshot 共用，失败返回错误由调用方记日志。

use std::io;
use std::path::Path;

/// 写临时文件后原子替换到 `dest`。目标可已存在。
pub fn write(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = dest.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    replace(&tmp, dest)
}

/// 用 `ReplaceFileW` / `MoveFileExW` 把 tmp 换到 dest；失败再退回 remove+rename。
fn replace(tmp: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            REPLACEFILE_WRITE_THROUGH,
        };

        fn wide(p: &Path) -> Vec<u16> {
            p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
        }

        let tmp_w = wide(tmp);
        let dest_w = wide(dest);
        let replaced = unsafe {
            if dest.exists() {
                let r = ReplaceFileW(
                    PCWSTR(dest_w.as_ptr()),
                    PCWSTR(tmp_w.as_ptr()),
                    PCWSTR::null(),
                    REPLACEFILE_WRITE_THROUGH,
                    None,
                    None,
                );
                if r.is_ok() {
                    return Ok(());
                }
            }
            MoveFileExW(
                PCWSTR(tmp_w.as_ptr()),
                PCWSTR(dest_w.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced.is_ok() {
            return Ok(());
        }
    }
    if dest.exists() {
        let _ = std::fs::remove_file(dest);
    }
    std::fs::rename(tmp, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kite-atomic-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn write_creates_then_overwrites_existing() {
        let dir = temp_dir("ow");
        let dest = dir.join("data.json");
        write(&dest, b"{\"v\":1}").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"{\"v\":1}");
        write(&dest, b"{\"v\":2}").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"{\"v\":2}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_to_unwritable_parent_returns_err() {
        // 不存在的父目录：write 失败且不 panic
        let dest = std::env::temp_dir()
            .join(format!("kite-atomic-missing-{}", std::process::id()))
            .join("nested")
            .join("data.json");
        assert!(write(&dest, b"x").is_err());
    }
}
