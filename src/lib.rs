//! Kite 启动入口：只做模块装配，业务不写在本文件。

pub mod app;
pub mod history;
pub mod log;
pub mod model;
pub mod search;
pub mod storage;
pub mod system;
pub mod ui;

/// 应用入口：原生 UI（iced + tiny-skia 软渲染，无 WebView2）。
pub fn run() {
    match system::elevation::relaunch_if_elevated() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("kite: elevated launch could not be converted to a user launch: {error}");
        }
    }

    if let Err(e) = ui::run() {
        eprintln!("kite: UI 启动失败: {e}");
        std::process::exit(1);
    }
}
