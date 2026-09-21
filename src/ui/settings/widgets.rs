//! 设置页共用控件：卡片、状态胶囊、开关与按钮。
//! 全面接入 ThemeTokens，支持深色与浅色自适应。

use iced::widget::{button, column, container, row, space::Space, text, text_input};
use iced::{alignment, border, color, Background, Border, Color, Element, Length};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::Message;

/// 统一 elevated 卡片：实体面板 + 1px 细微光边框 + 圆角。
pub(super) fn elevated_card<'a>(
    content: Element<'a, Message>,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding([14.0, 16.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        })
        .into()
}

/// 运行状态胶囊：坏态红字淡红底，停用灰字，其余中性/Accent。
pub(super) fn status_pill(
    label: &str,
    bad: bool,
    enabled: bool,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let (bg, fg) = if bad {
        if tokens.is_dark {
            (Color::from_rgba(0.9, 0.2, 0.2, 0.15), color!(0xEF_44_44))
        } else {
            (color!(0xFE_F2_F2), color!(0xDC_26_26))
        }
    } else if !enabled {
        if tokens.is_dark {
            (Color::from_rgba(1.0, 1.0, 1.0, 0.06), tokens.text_muted)
        } else {
            (color!(0xF3_F4_F6), tokens.text_muted)
        }
    } else {
        (
            Color {
                a: 0.12,
                ..tokens.accent
            },
            tokens.accent,
        )
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

/// 卡片：bg-elevated + 边框 + 行间 1px 分隔（通用设置页等使用）。
pub(super) fn flow_card(
    rows: Vec<Element<'static, Message>>,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let mut col = column![].width(Length::Fill);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            col = col.push(divider(tokens));
        }
        col = col.push(r);
    }
    container(col)
        .width(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
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
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    container(
        row![
            column![
                text(title)
                    .size(13.5)
                    .color(tokens.text_primary)
                    .font(name_font()),
                text(hint).size(12.0).color(tokens.text_muted)
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
    .into()
}

pub(super) fn divider(tokens: ThemeTokens) -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.border_window)),
            ..container::Style::default()
        })
        .into()
}

/// 开关：40×22 胶囊 + 白色滑块，自适应高对比度。
pub(super) fn toggle(
    checked: bool,
    on_press: Message,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let bg_color = if checked {
        tokens.accent
    } else if tokens.is_dark {
        Color::from_rgba(1.0, 1.0, 1.0, 0.16)
    } else {
        Color::from_rgba(0.0, 0.0, 0.0, 0.15)
    };
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
        background: Some(Background::Color(bg_color)),
        border: Border::default().rounded(999.0),
        ..button::Style::default()
    })
    .into()
}

pub(super) fn std_button(
    label: &'static str,
    on_press: Message,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    button(text(label).size(13.0).color(tokens.text_primary))
        .padding([7.0, 12.0])
        .on_press(on_press)
        .style(move |_t, status| button::Style {
            background: if status == button::Status::Hovered {
                Some(Background::Color(tokens.active_bg))
            } else {
                Some(Background::Color(tokens.bg_input))
            },
            text_color: tokens.text_primary,
            border: Border {
                color: tokens.border_window,
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
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let color = if danger {
        color!(0xDC_26_26)
    } else {
        tokens.text_muted
    };
    button(text(label).size(12.0).color(color))
        .padding([5.0, 9.0])
        .on_press(on_press)
        .style(move |_t, status| button::Style {
            background: if status == button::Status::Hovered {
                Some(Background::Color(tokens.active_bg))
            } else {
                None
            },
            text_color: color,
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(6.0),
            },
            ..button::Style::default()
        })
        .into()
}

pub(super) fn primary_button(
    label: String,
    on_press: Message,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    button(text(label).size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(on_press)
        .style(move |_t, _s| button::Style {
            background: Some(Background::Color(tokens.accent)),
            text_color: color!(0xFF_FF_FF),
            border: Border::default().rounded(8.0),
            ..button::Style::default()
        })
        .into()
}

/// 统一输入框样式
pub(super) fn text_input_style(
    tokens: ThemeTokens,
) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |_t, _s| text_input::Style {
        background: Background::Color(tokens.bg_input),
        border: Border {
            color: tokens.border_window,
            width: 1.0,
            radius: border::radius(8.0),
        },
        icon: tokens.text_muted,
        placeholder: tokens.text_muted,
        value: tokens.text_primary,
        selection: Color {
            a: 0.25,
            ..tokens.accent
        },
    }
}
