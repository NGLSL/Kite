//! 操作系统集成：图标、托盘事件、开机启动、Everything、浏览器、快捷键、音效、环境刷新。
//! 窗口与托盘的原生实现位于 ui 模块（iced 窗口 + tray-icon）。

pub mod autostart;
pub mod browsers;
pub mod env;
pub mod everything;
pub mod hotkey;
pub mod icons;
pub mod resources;
pub mod search_engine;
pub mod sound;
