//! Kite 启动入口：只做模块装配，业务不写在本文件。

pub mod app;
pub mod history;
pub mod log;
pub mod model;
pub mod plugin;
pub mod search;
pub mod storage;
pub mod system;
pub mod ui;

/// 应用入口：原生 UI（iced + tiny-skia 软渲染，无 WebView2）。
pub fn run() {
    // Initialize file logging before elevation handling so bootstrap failures
    // and a guarded de-elevation retry are visible even when the UI never starts.
    let data_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.kite.launcher");
    let _ = std::fs::create_dir_all(&data_dir);
    log::init(log::default_path_under(&data_dir));
    log::info(&format!(
        "entry version={} pid={} elevated-check",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));

    match system::elevation::relaunch_if_elevated() {
        Ok(true) => {
            log::info("elevated bootstrap exiting after child launch");
            return;
        }
        Ok(false) => {}
        Err(error) => {
            log::info(&format!(
                "elevated launch could not be converted to a user launch: {error}"
            ));
            eprintln!("kite: elevated launch could not be converted to a user launch: {error}");
        }
    }

    // 降权之后再 claim，避免提权 bootstrap 短暂占位误伤正常实例。
    if let Err(system::singleton::AlreadyRunning) = system::singleton::claim() {
        log::info("secondary launch exited after requesting activation");
        return;
    }

    if let Err(e) = ui::run() {
        eprintln!("kite: UI 启动失败: {e}");
        std::process::exit(1);
    }
}
