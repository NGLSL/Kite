//! 全局快捷键。

use iced::font::Weight;
use iced::widget::{button, column, container, row, text};
use iced::{alignment, border, color, Background, Border, Color, Element, Font, Length, Padding};

use super::super::font::ui_font;
use super::super::search_view::{BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED};
use super::super::{Message, State};
use super::widgets::{flow_card, flow_row};

const HOTKEY_PRESETS: [&str; 5] = [
    "Alt+Space",
    "Ctrl+Alt+Space",
    "Ctrl+Shift+K",
    "Alt+K",
    "Ctrl+Alt+A",
];

pub(super) fn hotkey_card(state: &State) -> Element<'_, Message> {
    let recording = state.hotkey_recording;
    let chip = container(
        text(state.hotkey_label.clone())
            .size(13.0)
            .color(TEXT)
            .font(Font {
                weight: Weight::Semibold,
                ..ui_font()
            }),
    )
    .padding([5.0, 10.0])
    .style(|_t| container::Style {
        background: Some(Background::Color(BG_PANEL)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: border::radius(6.0),
        },
        ..container::Style::default()
    });

    let change: Element<'_, Message> = button(
        text(if recording {
            "按下组合键…"
        } else {
            "更改"
        })
        .size(13.0)
        .color(if recording { color!(0xFF_FF_FF) } else { TEXT }),
    )
    .padding([7.0, 12.0])
    .on_press(Message::StartHotkeyRecord)
    .style(move |_t, _s| button::Style {
        background: if recording {
            Some(Background::Color(MARK))
        } else {
            Some(Background::Color(BG_PANEL))
        },
        text_color: if recording { color!(0xFF_FF_FF) } else { TEXT },
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: border::radius(8.0),
        },
        ..button::Style::default()
    })
    .into();

    let mut presets = row![].spacing(6.0);
    for preset in HOTKEY_PRESETS {
        let on = state.hotkey == preset;
        presets = presets.push(
            button(text(preset).size(12.0).color(if on { MARK } else { TEXT_MUTED }))
                .padding([5.0, 10.0])
                .on_press(Message::ApplyHotkey(preset.to_string()))
                .style(move |_t, _s| button::Style {
                    background: if on {
                        Some(Background::Color(Color { a: 0.10, ..MARK }))
                    } else {
                        Some(Background::Color(BG_PANEL))
                    },
                    text_color: if on { MARK } else { TEXT_MUTED },
                    border: Border {
                        color: if on {
                            Color { a: 0.45, ..MARK }
                        } else {
                            BORDER
                        },
                        width: 1.0,
                        radius: border::radius(999.0),
                    },
                    ..button::Style::default()
                }),
        );
    }

    flow_card(vec![
        flow_row(
            "全局快捷键",
            "按下组合键后立即生效".to_string(),
            row![chip, change]
                .spacing(8.0)
                .align_y(alignment::Alignment::Center)
                .into(),
        ),
        container(column![text("常用预设").size(12.0).color(TEXT_MUTED), presets].spacing(8.0))
            .width(Length::Fill)
            .padding(Padding {
                top: 12.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            })
            .into(),
    ])
}
