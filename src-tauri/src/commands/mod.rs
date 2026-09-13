//! React 与 Rust 的 IPC 面。本目录每个文件只做转发，业务逻辑放在同级业务模块。
//! lib.rs 的 generate_handler 按完整路径引用各命令（commands::search::search_apps 等）。

pub mod launch;
pub mod search;
pub mod settings;
