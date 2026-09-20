//! 统一的独立开发者工具窗样式。
//!
//! 工具本身只负责提供标题、操作按钮和内容；窗口壳层、间距、颜色与编辑器/结果框
//! 的视觉规则集中在这里，保证 JSON、Hash、Base64 等工具拥有相同的使用感受。

use iced::font::{Family, Font};
use iced::widget::{
    button as iced_button, column, container, mouse_area, row, scrollable, space::Space, text,
    text_editor,
};
use iced::{alignment, border, color, Background, Border, Color, Element, Length, Padding, Theme};

use super::font::{name_font, ui_font};
use super::search_view::{results_scroll_style_tokens, results_scrollbar};
use super::theme::ThemeTokens;
use super::Message;

/// 标题栏所需的最小信息。标题栏本身负责窗口拖拽和关闭按钮的统一行为。
pub(super) struct Header {
    pub icon: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub drag: Message,
    pub close: Message,
}

/// 提示条的语气。Hint 用于工具的常驻说明，Success/Error 用于一次性操作结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoteTone {
    Hint,
    Success,
    Error,
}

/// 生成完整的工具窗壳层。
///
/// 调用方只需要提供标题栏描述、工具栏、提示条和主体内容；窗口底色、标题分割线、
/// 内容边距以及工具栏到双栏主体的间距在此统一维护。
pub(super) fn window<'a>(
    tokens: ThemeTokens,
    header: Header,
    toolbar: Element<'a, Message>,
    note: Element<'a, Message>,
    panes: Element<'a, Message>,
) -> Element<'a, Message> {
    let title_bar = title_bar(header, tokens);
    let body = column![toolbar, note, panes]
        .spacing(10.0)
        .width(Length::Fill)
        .height(Length::Fill);

    let shell = column![
        title_bar,
        divider(tokens),
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: 12.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            }),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    container(shell)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(Background::Color(tokens.bg_window)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(0.0),
            },
            ..container::Style::default()
        })
        .into()
}

fn title_bar(header: Header, tokens: ThemeTokens) -> Element<'static, Message> {
    mouse_area(
        container(
            row![
                text(header.icon)
                    .size(14.0)
                    .color(tokens.accent)
                    .font(Font {
                        family: Family::Monospace,
                        ..ui_font()
                    }),
                text(header.title)
                    .size(13.5)
                    .font(name_font())
                    .color(tokens.text_primary),
                text(header.subtitle).size(11.0).color(tokens.text_muted),
                Space::new().width(Length::Fill),
                iced_button(text("×").size(16.0).color(tokens.text_muted))
                    .padding([4.0, 8.0])
                    .on_press(header.close)
                    .style(move |_theme, status| iced_button::Style {
                        background: if status == iced_button::Status::Hovered {
                            Some(Background::Color(tokens.active_bg))
                        } else {
                            None
                        },
                        text_color: if status == iced_button::Status::Hovered {
                            tokens.text_primary
                        } else {
                            tokens.text_muted
                        },
                        border: Border::default().rounded(6.0),
                        ..iced_button::Style::default()
                    }),
            ]
            .spacing(10.0)
            .align_y(alignment::Alignment::Center)
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(44.0)
        .align_y(alignment::Alignment::Center)
        .padding(Padding {
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
            left: 16.0,
        }),
    )
    .on_press(header.drag)
    .into()
}

fn divider(tokens: ThemeTokens) -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(move |_theme| container::Style {
            background: Some(Background::Color(tokens.border_window)),
            ..container::Style::default()
        })
        .into()
}

/// 统一工具栏布局。按钮由 [`button`] 创建，以便每个工具只声明自己的操作顺序。
pub(super) fn toolbar(buttons: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    row(buttons)
        .spacing(8.0)
        .align_y(alignment::Alignment::Center)
        .into()
}

/// 统一主按钮/次按钮样式。主按钮使用强调色，次按钮使用卡片底色和细边框。
pub(super) fn button(
    label: &'static str,
    message: Message,
    primary: bool,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let text_color = if primary {
        color!(0xFF_FF_FF)
    } else {
        tokens.text_primary
    };
    iced_button(text(label).size(12.5).color(text_color))
        .padding([7.0, 12.0])
        .on_press(message)
        .style(move |_theme, status| iced_button::Style {
            background: Some(Background::Color(if primary {
                tokens.accent
            } else if status == iced_button::Status::Hovered {
                tokens.active_bg
            } else {
                tokens.bg_elevated
            })),
            text_color,
            border: Border {
                color: if primary {
                    tokens.accent
                } else {
                    tokens.border_window
                },
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..iced_button::Style::default()
        })
        .into()
}

/// 空提示条，保留工具栏与双栏主体之间统一的布局槽位。
pub(super) fn no_note<'a>() -> Element<'a, Message> {
    Space::new().height(0.0).into()
}

/// 生成工具的提示条。Hint 是常驻的低强调说明；Success/Error 使用带底色的状态条。
pub(super) fn note(
    message: impl Into<String>,
    tone: NoteTone,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let message = message.into();
    match tone {
        NoteTone::Hint => text(message).size(11.0).color(tokens.text_muted).into(),
        NoteTone::Success | NoteTone::Error => {
            let is_error = matches!(tone, NoteTone::Error);
            let (background, foreground) = if is_error {
                if tokens.is_dark {
                    (Color::from_rgba(0.9, 0.2, 0.2, 0.15), color!(0xEF_44_44))
                } else {
                    (color!(0xFE_F2_F2), color!(0xDC_26_26))
                }
            } else if tokens.is_dark {
                (Color::from_rgba(0.1, 0.6, 0.6, 0.15), color!(0x14_B8_A6))
            } else {
                (color!(0xEC_FC_FF), color!(0x0F_76_6E))
            };
            container(
                text(message)
                    .size(12.0)
                    .color(foreground)
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .padding([6.0, 10.0])
            .style(move |_theme: &Theme| container::Style {
                background: Some(Background::Color(background)),
                border: Border::default().rounded(8.0),
                ..container::Style::default()
            })
            .into()
        }
    }
}

/// 统一左侧文本编辑器样式。
pub(super) fn editor<'a>(
    content: &'a text_editor::Content,
    placeholder: &'static str,
    on_action: fn(text_editor::Action) -> Message,
    font: Font,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    text_editor(content)
        .placeholder(placeholder)
        .on_action(on_action)
        .height(Length::Fill)
        .min_height(280.0)
        .size(13.0)
        .font(font)
        .padding(12.0)
        .style(move |_theme, status| text_editor::Style {
            background: Background::Color(tokens.bg_input),
            border: Border {
                color: if matches!(status, text_editor::Status::Focused { .. }) {
                    tokens.accent
                } else {
                    tokens.border_window
                },
                width: 1.0,
                radius: border::radius(8.0),
            },
            placeholder: tokens.text_muted,
            value: tokens.text_primary,
            selection: Color {
                a: 0.25,
                ..tokens.accent
            },
        })
        .into()
}

/// 给编辑器增加统一的字段标题。
pub(super) fn editor_pane<'a>(
    label: &'static str,
    editor: Element<'a, Message>,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    column![text(label).size(12.0).color(tokens.text_muted), editor]
        .spacing(6.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 统一右侧结果框样式。结果文本由调用方先复制为 String，保证返回元素不借用 State。
pub(super) fn result_pane(
    label: &'static str,
    result: String,
    empty: bool,
    font: Font,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let result_color = if empty {
        tokens.text_muted
    } else {
        tokens.text_primary
    };
    let result_text = text(result)
        .size(if empty { 12.0 } else { 13.0 })
        .font(font)
        .color(result_color)
        .width(Length::Fill);
    let result_box = container(
        scrollable(result_text)
            .direction(scrollable::Direction::Vertical(results_scrollbar()))
            .style(results_scroll_style_tokens(tokens))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(12.0)
    .style(move |_theme| container::Style {
        background: Some(Background::Color(tokens.bg_elevated)),
        border: Border {
            color: tokens.border_window,
            width: 1.0,
            radius: border::radius(8.0),
        },
        ..container::Style::default()
    });

    column![text(label).size(12.0).color(tokens.text_muted), result_box]
        .spacing(6.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 双栏布局的统一列间距和伸展行为。
pub(super) fn panes<'a>(
    left: Element<'a, Message>,
    right: Element<'a, Message>,
) -> Element<'a, Message> {
    row![left, right]
        .spacing(12.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
