//! 应用目录：扫描 Windows 入口并启动目标。

pub mod actions;
pub mod builtin;
mod control_panel;
mod launcher;
pub mod scanner;
pub mod snapshot;
pub mod uwp;
pub mod watch;
pub mod web;
mod windows_settings;

pub use launcher::launch;
pub use scanner::scan_apps;
