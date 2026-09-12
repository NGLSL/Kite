//! 捆绑资源定位（Everything64.dll / open.wav 等共用）。
//! 统一查找顺序，新增捆绑文件只需一个名字，不再各写一套 candidates。

use std::path::PathBuf;
use std::sync::OnceLock;

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// setup 时注入 Tauri 资源目录（Windows 上为 exe 所在目录）。
pub fn init(resource_dir: PathBuf) {
    let _ = RESOURCE_DIR.set(resource_dir);
}

/// 按优先级返回候选路径：
/// 资源目录 `resources/` 子目录（安装包与 dev 由 Tauri 保持的布局）
/// → 资源目录平级 → 编译期 crate `resources/`（dev 兜底）→ exe 目录。
pub fn candidates(name: &str) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = RESOURCE_DIR.get() {
        v.push(dir.join("resources").join(name));
        v.push(dir.join(name));
    }
    v.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(name),
    );
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("resources").join(name));
            v.push(dir.join(name));
        }
    }
    v
}

/// 从候选里取第一个真实存在的文件。
pub fn locate(name: &str) -> Option<PathBuf> {
    candidates(name).into_iter().find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_files_locatable() {
        init(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        assert!(locate("Everything64.dll").is_some(), "DLL 应可定位");
        assert!(locate("open.wav").is_some(), "音效应可定位");
        assert!(locate("no-such-bundled.bin").is_none());
    }
}
