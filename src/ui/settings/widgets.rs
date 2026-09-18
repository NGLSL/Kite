//! 设置页共用控件：卡片、状态胶囊、开关与按钮。

use iced::widget::{button, column, container, row, space::Space, text};
use iced::{alignment, border, color, Background, Border, Color, Element, Length};

use super::super::font::name_font;
use super::super::search_view::{BG_ELEVATED, BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED};
use super::super::Message;

/// 列表分区标题（弱化，不再塞进 flow 卡片）。
#[allow(dead_code)]
pub(super) fn section_title(title: &str) -> Element<'static, Message> {
    text(title.to_string())
        .size(13.0)
        .color(TEXT)
        .font(name_font())
        .into()
}

/// 统一 elevated 卡片：白底面板 + 描边 + 圆角，列表页与详情页共用。
pub(super) fn elevated_card(content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .width(Length::Fill)
        .padding([14.0, 16.0])
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        })
        .into()
}

/// 运行状态胶囊：坏态红字淡红底，停用灰字，其余中性。
pub(super) fn status_pill(label: &str, bad: bool, enabled: bool) -> Element<'static, Message> {
    let (bg, fg) = if bad {
        (color!(0xFE_F2_F2), color!(0xDC_26_26))
    } else if !enabled {
        (color!(0xF3_F4_F6), TEXT_MUTED)
    } else {
        (color!(0xEF_F6_FF), MARK)
    };
    container(text(label.to_string()).size(11.0).color(fg))
        .padding([2.0, 8.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(bg)),
            border: Border::default().rounded(999.0),
            ..container::Style::default()
        })
        .into()
}

/// 卡片：bg-elevated + 边框 + 行间 1px 分隔（通用设置页仍使用）。
pub(super) fn flow_card(rows: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    let mut col = column![].width(Length::Fill);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            col = col.push(divider());
        }
        col = col.push(r);
    }
    container(col)
        .width(Length::Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        })
        .into()
}

/// 一行 Flow 设置：左标题+说明，右控件。
pub(super) fn flow_row(
    title: &'static str,
    hint: String,
    control: Element<'static, Message>,
) -> Element<'static, Message> {
    container(
        row![
            column![
                text(title).size(13.5).color(TEXT).font(name_font()),
                text(hint).size(12.0).color(TEXT_MUTED)
            ]
            .spacing(2.0)
            .width(Length::Fill),
            control,
        ]
        .spacing(12.0)
        .align_y(alignment::Alignment::Center),
    )
    .width(Length::Fill)
    .padding([12.0, 14.0])
    .style(|_t| container::Style {
        border: Border {
            color: Color { a: 0.8, ..BORDER },
            width: 0.0,
            radius: border::radius(0.0),
        },
        ..container::Style::default()
    })
    .into()
}

pub(super) fn divider() -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(|_t| container::Style {
            background: Some(Background::Color(Color { a: 0.8, ..BORDER })),
            ..container::Style::default()
        })
        .into()
}

/// 开关：40×22 胶囊 + 白色滑块。
pub(super) fn toggle(checked: bool, on_press: Message) -> Element<'static, Message> {
    button(
        container(
            container(Space::new().width(14.0).height(14.0)).style(|_t| container::Style {
                background: Some(Background::Color(color!(0xFF_FF_FF))),
                border: Border::default().rounded(999.0),
                ..container::Style::default()
            }),
        )
        .width(40.0)
        .height(22.0)
        .padding(2.0)
        .align_x(if checked {
            alignment::Alignment::End
        } else {
            alignment::Alignment::Start
        })
        .align_y(alignment::Alignment::Center),
    )
    .on_press(on_press)
    .style(move |_t, _s| button::Style {
        background: Some(Background::Color(if checked {
            MARK
        } else {
            Color { a: 0.18, ..TEXT }
        })),
        border: Border::default().rounded(999.0),
        ..button::Style::default()
    })
    .into()
}

pub(super) fn std_button(label: &'static str, on_press: Message) -> Element<'static, Message> {
    button(text(label).size(13.0).color(TEXT))
        .padding([7.0, 12.0])
        .on_press(on_press)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(BG_PANEL)),
            text_color: TEXT,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..button::Style::default()
        })
        .into()
}

pub(super) fn ghost_button(
    label: String,
    on_press: Message,
    danger: bool,
) -> Element<'static, Message> {
    let color = if danger {
        color!(0xDC_26_26)
    } else {
        TEXT_MUTED
    };
    button(text(label).size(12.0).color(color))
        .padding([5.0, 9.0])
        .on_press(on_press)
        .style(move |_t, _s| button::Style {
            background: None,
            text_color: color,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(6.0),
            },
            ..button::Style::default()
        })
        .into()
}

pub(super) fn primary_button(label: String, on_press: Message) -> Element<'static, Message> {
    button(text(label).size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(on_press)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(MARK)),
            text_color: color!(0xFF_FF_FF),
            border: Border::default().rounded(8.0),
            ..button::Style::default()
        })
        .into()
}
