//! Hash 独立工具窗：输入 UTF-8 文本，实时计算 SHA-256。

use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space::Space, text, text_editor,
};
use iced::{alignment, border, Background, Border, Color, Element, Length, Padding};
use sha2::{Digest, Sha256};

use super::font::name_font;
use super::search_view::{results_scroll_style_tokens, results_scrollbar};
use super::{Message, State};

pub(super) const TOOL_W: f32 = 760.0;
pub(super) const TOOL_H: f32 = 460.0;

pub(super) fn is_native_hash_activation(act: &crate::plugin::Activation) -> bool {
    act.plugin_id == "com.kite.devtools" && act.provider_id == "hash"
}

pub(super) fn input_text(content: &text_editor::Content) -> String {
    content.text()
}

pub(super) fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let tokens = state.theme_tokens();
    let title = mouse_area(
        container(
            row![
                text("#").size(17.0).color(tokens.accent),
                text("Hash 工具")
                    .size(13.5)
                    .font(name_font())
                    .color(tokens.text_primary),
                text("SHA-256 · UTF-8 文本")
                    .size(11.0)
                    .color(tokens.text_muted),
                Space::new().width(Length::Fill),
                button(text("×").size(16.0).color(tokens.text_muted))
                    .padding([4.0, 8.0])
                    .on_press(Message::PluginCloseHashTool),
            ]
            .spacing(10.0)
            .align_y(alignment::Alignment::Center),
        )
        .width(Length::Fill)
        .height(44.0)
        .align_y(alignment::Alignment::Center)
        .padding([0.0, 16.0]),
    )
    .on_press(Message::HashToolDrag);

    let toolbar = row![
        button("从剪贴板粘贴").on_press(Message::HashToolPaste),
        button("复制 SHA-256").on_press(Message::HashToolCopyResult),
        button("清空").on_press(Message::HashToolClear),
    ]
    .spacing(8.0);

    let editor = text_editor(&state.hash_editor)
        .placeholder("在这里输入或粘贴要计算 Hash 的文本…")
        .on_action(Message::HashToolEdit)
        .height(Length::Fill)
        .padding(12.0)
        .size(13.0)
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
        });

    let result = if state.hash_result.is_empty() {
        "输入文本后自动计算；空文本不显示结果".to_string()
    } else {
        state.hash_result.clone()
    };
    let result_color = if state.hash_result.is_empty() {
        tokens.text_muted
    } else {
        tokens.text_primary
    };
    let result_box = container(
        scrollable(
            text(result)
                .size(14.0)
                .font(iced::Font::MONOSPACE)
                .color(result_color),
        )
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

    let panes = row![
        column![text("原始文本").size(12.0).color(tokens.text_muted), editor]
            .spacing(6.0)
            .width(Length::Fill)
            .height(Length::Fill),
        column![
            text("SHA-256 结果").size(12.0).color(tokens.text_muted),
            result_box
        ]
        .spacing(6.0)
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .spacing(12.0)
    .width(Length::Fill)
    .height(Length::Fill);

    let note = state
        .hash_tool_note
        .as_deref()
        .unwrap_or("输入变化后自动计算 · 结果为 64 位十六进制字符串");
    let body = column![
        toolbar,
        text(note).size(11.0).color(tokens.text_muted),
        panes
    ]
    .spacing(10.0)
    .width(Length::Fill)
    .height(Length::Fill);
    let shell = column![
        title,
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: 12.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_value_and_editor_newline() {
        let editor = text_editor::Content::with_text("abc");
        assert_eq!(input_text(&editor), "abc");
        assert_eq!(
            sha256_hex(&input_text(&editor)),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            input_text(&text_editor::Content::with_text("abc\n")),
            "abc\n"
        );
    }
}
