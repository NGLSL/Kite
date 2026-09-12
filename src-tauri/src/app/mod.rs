//! 应用目录：扫描 Windows 入口并启动目标。

pub mod builtin;
mod launcher;
pub mod scanner;
pub mod uwp;
pub mod web;

pub use launcher::launch;
pub use scanner::scan_apps;
