//! 搜索引擎：预设 chips + 自定义模板 + 网页搜索快捷键。

use iced::widget::{button, column, container, row, text, text_input};
use iced::{border, Background, Border, Color, Element, Length, Padding};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::{Message, State};
use super::widgets::divider;

pub(super) fn search_engine_card<'a>(
    state: &'a State,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    let engine = state.search_engine.clone();
    let mut chip_vec: Vec<Element<'static, Message>> = Vec::new();
    for preset in crate::system::search_engine::ENGINE_PRESETS {
        let on = engine == preset.id;
        chip_vec.push(
            button(text(preset.label).size(13.0).color(if on {
                tokens.accent
            } else {
                tokens.text_muted
            }))
            .padding([7.0, 14.0])
            .on_press(Message::SetSearchEngine(preset.id.to_string()))
            .style(move |_t, _s| button::Style {
                background: if on {
                    Some(Background::Color(Color {
                        a: 0.12,
                        ..tokens.accent
                    }))
                } else {
                    Some(Background::Color(tokens.bg_input))
                },
                text_color: if on {
                    tokens.accent
                } else {
                    tokens.text_muted
                },
                border: Border {
                    color: if on {
                        Color {
                            a: 0.45,
                            ..tokens.accent
                        }
                    } else {
                        tokens.border_window
                    },
                    width: 1.0,
                    radius: border::radius(999.0),
                },
                ..button::Style::default()
            })
            .into(),
        );
    }
    let chips = row(chip_vec).spacing(8.0).wrap();

    let mut rows: Vec<Element<'static, Message>> = vec![
        container(
            column![
                text("默认引擎")
                    .size(13.5)
                    .color(tokens.text_primary)
                    .font(name_font()),
                text("网页搜索建议使用的引擎；「自动」跟随浏览器默认搜索引擎")
                    .size(12.0)
                    .color(tokens.text_muted),
            ]
            .width(Length::Fill)
            .padding(Padding {
                top: 14.0,
                right: 14.0,
                bottom: 4.0,
                left: 14.0,
            }),
        )
        .into(),
        container(chips)
            .width(Length::Fill)
            .padding(Padding {
                top: 8.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            })
            .into(),
    ];

    if engine == "custom" {
        let custom = state.search_engine_custom.clone();
        rows.push(
            container(
                column![
                    text("URL 模板")
                        .size(13.5)
                        .color(tokens.text_primary)
                        .font(name_font()),
                    text("必须包含 {searchTerms} 或 %s，将替换为已编码关键词")
                        .size(12.0)
                        .color(tokens.text_muted),
                    text_input("https://www.google.com/search?q={searchTerms}", &custom)
                        .on_input(Message::SearchEngineCustomChanged)
                        .size(13.0)
                        .padding([8.0, 10.0])
                        .width(Length::Fill)
                        .style(move |_t, _s| text_input::Style {
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
                        }),
                ]
                .spacing(6.0),
            )
            .width(Length::Fill)
            .padding(Padding {
                top: 4.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            })
            .into(),
        );
    }

    let mut web_chip_vec: Vec<Element<'static, Message>> = Vec::new();
    for preset in crate::system::hotkey::WEB_SEARCH_HOTKEY_PRESETS {
        let on = state.web_search_hotkey.eq_ignore_ascii_case(preset);
        web_chip_vec.push(
            button(text(*preset).size(12.0).color(if on {
                tokens.accent
            } else {
                tokens.text_muted
            }))
            .padding([6.0, 12.0])
            .on_press(Message::SetWebSearchHotkey(preset.to_string()))
            .style(move |_t, _s| button::Style {
                background: if on {
                    Some(Background::Color(Color {
                        a: 0.12,
                        ..tokens.accent
                    }))
                } else {
                    Some(Background::Color(tokens.bg_input))
                },
                text_color: if on {
                    tokens.accent
                } else {
                    tokens.text_muted
                },
                border: Border {
                    color: if on {
                        Color {
                            a: 0.45,
                            ..tokens.accent
                        }
                    } else {
                        tokens.border_window
                    },
                    width: 1.0,
                    radius: border::radius(999.0),
                },
                ..button::Style::default()
            })
            .into(),
        );
    }
    let web_chips = row(web_chip_vec).spacing(8.0).wrap();
    rows.push(
        container(
            column![
                text("网页搜索快捷键")
                    .size(13.5)
                    .color(tokens.text_primary)
                    .font(name_font()),
                text("搜索窗口内用浏览器打开当前关键词；仅在窗口内生效")
                    .size(12.0)
                    .color(tokens.text_muted),
                web_chips,
            ]
            .spacing(8.0),
        )
        .width(Length::Fill)
        .padding(Padding {
            top: 12.0,
            right: 14.0,
            bottom: 14.0,
            left: 14.0,
        })
        .into(),
    );

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
