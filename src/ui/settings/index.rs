//! 索引与便携目录。

use iced::widget::{button, column, container, row, text, text_input};
use iced::{border, color, Background, Border, Color, Element, Length};

use super::super::font::name_font;
use super::super::search_view::{BG_ELEVATED, BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED};
use super::super::{Message, State};
use super::widgets::{flow_card, flow_row, std_button};

pub(super) fn index_card(state: &State) -> Element<'_, Message> {
    let n = state.index.lock().map(|g| g.apps.len()).unwrap_or(0);
    let health = crate::app::index_health::load();
    let now = crate::storage::now_ts().max(0) as u64;
    let health_label = health.summary(now);
    let health_status = if crate::app::index_health::is_degraded(&health) {
        "需关注"
    } else {
        "正常"
    };
    let summary = flow_card(vec![
        flow_row(
            "重新扫描应用",
            "更新系统入口、软件注册信息和便携目录".to_string(),
            std_button("重新扫描", Message::Rescan),
        ),
        flow_row(
            "当前索引",
            "快速扫描 + AppsFolder 后台补齐".to_string(),
            text(format!("{n} 条")).size(13.0).color(TEXT).into(),
        ),
        flow_row(
            "索引健康",
            health_label,
            text(health_status).size(13.0).color(TEXT_MUTED).into(),
        ),
    ]);

    let input = text_input(r"D:\Portable Apps", &state.portable_dir_input)
        .on_input(Message::PortableDirInputChanged)
        .on_submit(Message::AddPortableDir)
        .padding([7.0, 10.0])
        .width(Length::Fill)
        .style(|_t, _s| text_input::Style {
            background: Background::Color(BG_PANEL),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(8.0),
            },
            icon: TEXT_MUTED,
            placeholder: TEXT_MUTED,
            value: TEXT,
            selection: Color { a: 0.25, ..MARK },
        });
    let add = button(text("添加").size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(Message::AddPortableDir)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(MARK)),
            text_color: color!(0xFF_FF_FF),
            border: Border::default().rounded(8.0),
            ..button::Style::default()
        });
    let mut directories = column![
        text("便携软件目录")
            .size(13.0)
            .color(TEXT)
            .font(name_font()),
        text("加入后扫描目录内的应用入口；修改会自动重建索引。")
            .size(12.0)
            .color(TEXT_MUTED),
        row![input, add].spacing(8.0),
    ]
    .spacing(8.0);
    for (index, path) in state.portable_dirs.iter().enumerate() {
        directories = directories.push(
            row![
                text(path.clone())
                    .size(12.0)
                    .color(TEXT)
                    .width(Length::Fill),
                button(text("移除").size(12.0).color(TEXT_MUTED))
                    .padding([4.0, 7.0])
                    .on_press(Message::RemovePortableDir(index))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: TEXT_MUTED,
                        border: Border::default().rounded(6.0),
                        ..button::Style::default()
                    }),
            ]
            .spacing(8.0)
            .align_y(iced::alignment::Alignment::Center),
        );
    }
    let directory_card = container(directories)
        .width(Length::Fill)
        .padding([12.0, 14.0])
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        });

    column![summary, directory_card]
        .spacing(12.0)
        .width(Length::Fill)
        .into()
}
