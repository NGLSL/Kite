//! 关于：版本、更新与仓库链接。

use iced::widget::{button, text};
use iced::{border, color, Background, Border, Element};

use super::super::theme::ThemeTokens;
use super::super::{Message, State};
use super::widgets::{flow_card, flow_row, std_button};

pub(super) fn about_card<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let hint = match &state.update_status {
        None => "对比 GitHub 最新发布版本".to_string(),
        Some(Ok(latest)) if latest == "latest" => "已是最新版本".to_string(),
        Some(Ok(latest)) => format!("发现新版本 {latest}"),
        Some(Err(e)) => format!("更新失败：{e}"),
    };
    let available = matches!(
        &state.update_status,
        Some(Ok(latest)) if latest != "latest"
    );

    let control: Element<'static, Message> = if available {
        let label = if state.update_checking {
            "下载中…"
        } else if state.update_asset.is_some() {
            "下载并安装"
        } else {
            "查看发布页"
        };
        let button = button(text(label).size(13.0))
            .padding([7.0, 12.0])
            .style(move |_t, _s| button::Style {
                background: Some(Background::Color(tokens.accent)),
                text_color: color!(0xFF_FF_FF),
                border: Border {
                    color: tokens.accent,
                    width: 1.0,
                    radius: border::radius(8.0),
                },
                ..button::Style::default()
            });
        if state.update_checking {
            button.into()
        } else {
            let action = if state.update_asset.is_some() {
                Message::DownloadUpdate
            } else {
                Message::OpenReleases
            };
            button.on_press(action).into()
        }
    } else {
        std_button(
            if state.update_checking {
                "检查中…"
            } else {
                "检查"
            },
            Message::CheckUpdate,
            tokens,
        )
    };

    flow_card(
        vec![
            flow_row(
                "Kite",
                "轻量 Windows 启动器".to_string(),
                text(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .size(12.0)
                    .color(tokens.text_muted)
                    .into(),
                tokens,
            ),
            flow_row("检查更新", hint, control, tokens),
            flow_row(
                "最新发布",
                "在 GitHub 下载官方安装包".to_string(),
                std_button("打开发布页", Message::OpenReleases, tokens),
                tokens,
            ),
            flow_row(
                "GitHub 仓库",
                "https://github.com/NGLSL/Kite".to_string(),
                std_button("访问仓库", Message::OpenRepository, tokens),
                tokens,
            ),
        ],
        tokens,
    )
}
