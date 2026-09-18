//! JSON 独立工具窗：真正的第二个 OS 窗口，不改主启动器窗口。
//!
//! 产品形态：选择 JSON 工具或搜索 `json` 后弹出独立工具窗——
//! 标题栏 + 双栏（左原始 / 右结果）；关闭只销毁工具窗，主窗保持原样。

use iced::font::{Family, Font};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space::Space, text, text_editor,
};
use iced::{alignment, border, color, Background, Border, Color, Element, Length, Padding, Theme};

use super::font::{name_font, ui_font};
use super::search_view::{results_scroll_style_tokens, results_scrollbar};
use super::theme::ThemeTokens;
use super::{Message, State};

/// 工具窗逻辑尺寸（独立于启动器 640×420 / 设置 720×520）。
pub(super) const TOOL_W: f32 = 880.0;
pub(super) const TOOL_H: f32 = 560.0;

/// 搜索列表：JSON 工具条目；Enter 打开。
pub(super) const CONFIRM_RESULT_ID: &str = "kite:json-tool-confirm";

/// 官方 DevTools 的 json Provider：宿主原生工具窗，需二次确认后打开。
pub(super) fn is_native_json_activation(act: &crate::plugin::Activation) -> bool {
    act.plugin_id == "com.kite.devtools" && act.provider_id == "json"
}

/// 解析/序列化 JSON：pretty 或 minify。
pub(super) fn transform_json(raw: &str, minify: bool) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请在左侧粘贴或输入 JSON".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| format!("JSON 无效：{e}"))?;
    if minify {
        serde_json::to_string(&value).map_err(|e| format!("序列化失败：{e}"))
    } else {
        serde_json::to_string_pretty(&value).map_err(|e| format!("序列化失败：{e}"))
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let tokens = state.theme_tokens();
    let title_bar = mouse_area(
        container(
            row![
                text("{ }")
                    .size(14.0)
                    .color(tokens.accent)
                    .font(Font {
                        family: Family::Monospace,
                        ..ui_font()
                    }),
                text("JSON 工具")
                    .size(13.5)
                    .font(name_font())
                    .color(tokens.text_primary),
                text("开发者工具 · 原生独立窗")
                    .size(11.0)
                    .color(tokens.text_muted),
                Space::new().width(Length::Fill),
                button(text("×").size(16.0).color(tokens.text_muted))
                    .padding([4.0, 8.0])
                    .on_press(Message::PluginCloseJsonTool)
                    .style(move |_t, status| button::Style {
                        background: if status == button::Status::Hovered {
                            Some(Background::Color(tokens.active_bg))
                        } else {
                            None
                        },
                        text_color: if status == button::Status::Hovered {
                            tokens.text_primary
                        } else {
                            tokens.text_muted
                        },
                        border: Border::default().rounded(6.0),
                        ..button::Style::default()
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
    .on_press(Message::JsonToolDrag);

    let toolbar = row![
        tool_btn("从剪贴板粘贴", Message::JsonToolPaste, false, tokens),
        tool_btn("格式化", Message::JsonToolFormat, true, tokens),
        tool_btn("压缩", Message::JsonToolMinify, false, tokens),
        tool_btn("复制结果", Message::JsonToolCopyResult, false, tokens),
        tool_btn("清空", Message::JsonToolClear, false, tokens),
    ]
    .spacing(8.0)
    .align_y(alignment::Alignment::Center);

    let note: Element<'_, Message> = match &state.json_tool_note {
        Some((is_err, msg)) => {
            let (bg, fg) = if *is_err {
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
            container(text(msg.clone()).size(12.0).color(fg).width(Length::Fill))
                .width(Length::Fill)
                .padding([6.0, 10.0])
                .style(move |_t: &Theme| container::Style {
                    background: Some(Background::Color(bg)),
                    border: Border::default().rounded(8.0),
                    ..container::Style::default()
                })
                .into()
        }
        None => Space::new().height(0.0).into(),
    };

    let mono = Font {
        family: Family::Name("Consolas"),
        ..ui_font()
    };

    let left = column![
        text("原始 JSON").size(12.0).color(tokens.text_muted),
        text_editor(&state.json_editor)
            .placeholder("粘贴或输入 JSON…")
            .on_action(Message::JsonToolEdit)
            .height(Length::Fill)
            .min_height(280.0)
            .size(13.0)
            .font(mono)
            .padding(12.0)
            .style(move |_t, status| text_editor::Style {
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
            }),
    ]
    .spacing(6.0)
    .width(Length::Fill)
    .height(Length::Fill);

    let result_text = if state.json_result.is_empty() {
        text("结果会显示在这里")
            .size(12.0)
            .color(tokens.text_muted)
            .width(Length::Fill)
    } else {
        text(state.json_result.clone())
            .size(13.0)
            .font(mono)
            .color(tokens.text_primary)
            .width(Length::Fill)
    };

    let right = column![
        text("结果").size(12.0).color(tokens.text_muted),
        container(
            scrollable(result_text)
                .direction(scrollable::Direction::Vertical(results_scrollbar()))
                .style(results_scroll_style_tokens(tokens))
                .height(Length::Fill)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..container::Style::default()
        }),
    ]
    .spacing(6.0)
    .width(Length::Fill)
    .height(Length::Fill);

    let panes = row![left, right]
        .spacing(12.0)
        .width(Length::Fill)
        .height(Length::Fill);

    let body = column![toolbar, note, panes]
        .spacing(10.0)
        .width(Length::Fill)
        .height(Length::Fill);

    let shell = column![
        title_bar,
        hdivider(tokens),
        container(body).width(Length::Fill).height(Length::Fill).padding(
            Padding {
                top: 12.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            }
        ),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    container(shell)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t| container::Style {
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

fn hdivider(tokens: ThemeTokens) -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.border_window)),
            ..container::Style::default()
        })
        .into()
}

fn tool_btn(
    label: &'static str,
    msg: Message,
    primary: bool,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let text_color = if primary {
        color!(0xFF_FF_FF)
    } else {
        tokens.text_primary
    };
    button(text(label).size(12.5).color(text_color))
        .padding([7.0, 12.0])
        .on_press(msg)
        .style(move |_t, status| button::Style {
            background: Some(Background::Color(if primary {
                tokens.accent
            } else if status == button::Status::Hovered {
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
            ..button::Style::default()
        })
        .into()
}
