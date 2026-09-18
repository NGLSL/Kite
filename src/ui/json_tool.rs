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
use super::search_view::{
    results_scroll_style, results_scrollbar, BG_ELEVATED, BG_PANEL, BORDER, WINDOW_BORDER, MARK,
    TEXT, TEXT_MUTED,
};
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

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let title_bar = mouse_area(
        container(
            row![
                text("JSON 工具")
                    .size(14.0)
                    .color(TEXT)
                    .font(name_font()),
                text("左原始 · 右格式化/压缩")
                    .size(11.5)
                    .color(TEXT_MUTED),
                Space::new().width(Length::Fill),
                button(text("×").size(16.0).color(TEXT_MUTED))
                    .padding([4.0, 10.0])
                    .on_press(Message::PluginCloseJsonTool)
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: TEXT_MUTED,
                        border: Border::default().rounded(8.0),
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
        tool_btn("从剪贴板粘贴", Message::JsonToolPaste, false),
        tool_btn("格式化", Message::JsonToolFormat, true),
        tool_btn("压缩", Message::JsonToolMinify, false),
        tool_btn("复制结果", Message::JsonToolCopyResult, false),
        tool_btn("清空", Message::JsonToolClear, false),
    ]
    .spacing(8.0)
    .align_y(alignment::Alignment::Center);

    let note: Element<'_, Message> = match &state.json_tool_note {
        Some((is_err, msg)) => {
            let (bg, fg) = if *is_err {
                (color!(0xFE_F2_F2), color!(0xDC_26_26))
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
        text("原始 JSON").size(12.0).color(TEXT_MUTED),
        text_editor(&state.json_editor)
            .placeholder("粘贴或输入 JSON…")
            .on_action(Message::JsonToolEdit)
            .height(Length::Fill)
            .min_height(280.0)
            .size(13.0)
            .font(mono)
            .padding(12.0)
            .style(|_t, status| text_editor::Style {
                background: Background::Color(BG_PANEL),
                border: Border {
                    color: if matches!(status, text_editor::Status::Focused { .. }) {
                        MARK
                    } else {
                        BORDER
                    },
                    width: 1.0,
                    radius: border::radius(8.0),
                },
                placeholder: TEXT_MUTED,
                value: TEXT,
                selection: Color { a: 0.25, ..MARK },
            }),
    ]
    .spacing(6.0)
    .width(Length::Fill)
    .height(Length::Fill);

    let result_text = if state.json_result.is_empty() {
        text("结果会显示在这里")
            .size(12.0)
            .color(TEXT_MUTED)
            .width(Length::Fill)
    } else {
        text(state.json_result.clone())
            .size(13.0)
            .font(mono)
            .color(TEXT)
            .width(Length::Fill)
    };

    let right = column![
        text("结果").size(12.0).color(TEXT_MUTED),
        container(
            scrollable(result_text)
                .direction(scrollable::Direction::Vertical(results_scrollbar()))
                .style(results_scroll_style)
                .height(Length::Fill)
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12.0)
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            border: Border {
                color: BORDER,
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
        hdivider(),
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
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_PANEL)),
            border: Border {
                color: WINDOW_BORDER,
                width: 1.0,
                radius: border::radius(0.0),
            },
            ..container::Style::default()
        })
        .into()
}

fn hdivider() -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(|_t| container::Style {
            background: Some(Background::Color(BORDER)),
            ..container::Style::default()
        })
        .into()
}

fn tool_btn(label: &'static str, msg: Message, primary: bool) -> Element<'static, Message> {
    let text_color = if primary {
        color!(0xFF_FF_FF)
    } else {
        TEXT
    };
    button(text(label).size(12.5).color(text_color))
        .padding([7.0, 12.0])
        .on_press(msg)
        .style(move |_t, _s| button::Style {
            background: Some(Background::Color(if primary {
                MARK
            } else {
                BG_ELEVATED
            })),
            text_color,
            border: Border {
                color: if primary { MARK } else { BORDER },
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..button::Style::default()
        })
        .into()
}
