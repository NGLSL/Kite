//! 应用目录：扫描 Windows 入口并启动目标。

mod launcher;
pub mod scanner;
pub mod uwp;

pub use launcher::launch;
pub use scanner::{fill_missing_icons, scan_apps};
