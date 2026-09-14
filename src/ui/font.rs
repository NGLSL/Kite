//! 为中文界面选择已安装的字体族；Iced 仍负责逐字形回退与绘制。

use std::sync::OnceLock;
use std::time::Instant;

use fontdb::Database;
use iced::font::{Family, Font, Weight};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
};

static UI_FAMILY: OnceLock<Option<String>> = OnceLock::new();

/// 全局默认 UI 字体。首次调用时读取系统字体族并缓存选择结果。
pub(crate) fn ui_font() -> Font {
    match UI_FAMILY.get_or_init(pick_ui_family).as_deref() {
        Some(family) => Font {
            family: Family::Name(family),
            ..Font::DEFAULT
        },
        None => Font::DEFAULT,
    }
}

/// 应用名和标题沿用同一字族，仅调整字重。
pub(crate) fn name_font() -> Font {
    Font {
        weight: Weight::Medium,
        ..ui_font()
    }
}

fn pick_ui_family() -> Option<String> {
    let started = Instant::now();
    let mut db = Database::new();
    db.load_system_fonts();
    let message_font = system_message_font();

    // Kite 的界面为简体中文：优先已安装的 Noto；否则使用 Windows 的
    // 消息字体。消息字体不可用时，再选常见的简体中文系统字体。
    let candidates = ["Noto Sans SC", message_font.as_deref().unwrap_or("")];
    let selected = candidates
        .into_iter()
        .chain(["Microsoft YaHei UI", "Microsoft YaHei", "DengXian"])
        .find(|name| !name.is_empty() && has_family(&db, name))
        .map(str::to_owned);

    crate::log::info(&format!(
        "ui font requested={} system_message_font={} font_faces={} scan_ms={}",
        selected.as_deref().unwrap_or("SansSerif"),
        message_font.as_deref().unwrap_or("unavailable"),
        db.len(),
        started.elapsed().as_millis(),
    ));
    selected
}

fn has_family(db: &Database, family: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
    })
}

fn system_message_font() -> Option<String> {
    let mut metrics = NONCLIENTMETRICSW::default();
    metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
            Default::default(),
        )
        .ok()?;
    }
    let face = &metrics.lfMessageFont.lfFaceName;
    let end = face.iter().position(|&ch| ch == 0).unwrap_or(face.len());
    let name = String::from_utf16_lossy(&face[..end]);
    (!name.is_empty()).then_some(name)
}
