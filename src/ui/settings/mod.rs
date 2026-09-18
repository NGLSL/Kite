//! 设置页：壳层（导航/标题/toast）+ 分区卡片 + 共用控件。
//! 全面接入 ThemeTokens，支持深浅色无缝自适应与即时换肤。

mod about;
mod alias;
mod general;
mod hotkey;
mod index;
mod plugins;
mod search_engine;
mod widgets;

use iced::font::Weight;
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space::Space, text,
};
use iced::{alignment, border, color, Background, Border, Color, Element, Font, Length, Padding};

use super::font::ui_font;
use super::theme::ThemeTokens;
use super::{Message, Section, State};

/// 设置页入口：左导航 + 右内容区 + 底部 toast。
pub fn settings_view(state: &State) -> Element<'_, Message> {
    let tokens = state.theme_tokens();
    let nav = nav_panel(state, tokens);
    let main = column![
        header(state.settings_section, tokens),
        scrollable(
            container(body(state, tokens))
                .width(Length::Fill)
                .padding(Padding {
                    top: 12.0,
                    right: 14.0,
                    bottom: 16.0,
                    left: 14.0,
                })
        )
        .direction(scrollable::Direction::Vertical(
            super::search_view::results_scrollbar(),
        ))
        .style(super::search_view::results_scroll_style_tokens(tokens))
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let root = container(row![nav, vdivider(tokens), main])
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
        });

    match &state.flash {
        Some(msg) => {
            let root: Element<'_, Message> = root.into();
            iced::widget::stack![root, toast(msg, tokens)].into()
        }
        None => root.into(),
    }
}

fn body<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    match state.settings_section {
        Section::General => general::general_card(state, tokens),
        Section::SearchEngine => search_engine::search_engine_card(state, tokens),
        Section::Hotkey => hotkey::hotkey_card(state, tokens),
        Section::Alias => alias::alias_card(state, tokens),
        Section::Index => index::index_card(state, tokens),
        Section::Plugins => plugins::plugins_card(state, tokens),
        Section::About => about::about_card(state, tokens),
    }
}

fn nav_panel<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let logo = container(text("K").size(15.0).color(color!(0xFF_FF_FF)).font(Font {
        weight: Weight::Bold,
        ..ui_font()
    }))
    .width(32.0)
    .height(32.0)
    .align_x(alignment::Alignment::Center)
    .align_y(alignment::Alignment::Center)
    .style(move |_t| container::Style {
        background: Some(Background::Color(tokens.accent)),
        border: Border::default().rounded(8.0),
        ..container::Style::default()
    });

    let brand = row![
        logo,
        column![
            text("Kite")
                .size(13.0)
                .font(super::font::name_font())
                .color(tokens.text_primary),
            text("设置").size(11.0).color(tokens.text_muted),
        ]
        .spacing(1.0)
        .width(Length::Fill),
        button(text("×").size(16.0).color(tokens.text_muted))
            .padding([4.0, 8.0])
            .on_press(Message::CloseSettings)
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
    .align_y(alignment::Alignment::Center);

    let mut nav_list = column![].spacing(2.0).padding(8.0);
    for section in Section::ALL {
        let on = state.settings_section == section;
        let bg_color = if on {
            tokens.active_bg
        } else {
            Color::TRANSPARENT
        };
        let text_color = if on {
            tokens.accent
        } else {
            tokens.text_muted
        };
        let border_color = if on {
            tokens.active_border
        } else {
            Color::TRANSPARENT
        };

        nav_list = nav_list.push(
            button(
                text(section.label())
                    .size(13.0)
                    .color(text_color)
                    .font(if on {
                        Font {
                            weight: Weight::Semibold,
                            ..ui_font()
                        }
                    } else {
                        ui_font()
                    }),
            )
            .width(Length::Fill)
            .padding([8.0, 10.0])
            .on_press(Message::SettingsSection(section))
            .style(move |_t, status| button::Style {
                background: if on {
                    Some(Background::Color(bg_color))
                } else if status == button::Status::Hovered {
                    Some(Background::Color(tokens.active_bg))
                } else {
                    None
                },
                text_color: if on {
                    text_color
                } else if status == button::Status::Hovered {
                    tokens.text_primary
                } else {
                    tokens.text_muted
                },
                border: Border {
                    color: border_color,
                    width: 1.0,
                    radius: border::radius(8.0),
                },
                ..button::Style::default()
            }),
        );
    }

    let nav_bg = if tokens.is_dark {
        tokens.bg_input
    } else {
        tokens.bg_elevated
    };

    container(column![
        mouse_area(
            container(brand)
                .width(Length::Fill)
                .height(53.0)
                .align_y(alignment::Alignment::Center)
                .padding(Padding {
                    top: 0.0,
                    right: 12.0,
                    bottom: 0.0,
                    left: 12.0
                })
        )
        .on_press(Message::DragWindow),
        divider(tokens),
        nav_list,
    ])
    .width(168.0)
    .height(Length::Fill)
    .style(move |_t| container::Style {
        background: Some(Background::Color(nav_bg)),
        border: Border {
            color: tokens.border_window,
            width: 0.0,
            radius: border::radius(0.0),
        },
        ..container::Style::default()
    })
    .into()
}

fn header(section: Section, tokens: ThemeTokens) -> Element<'static, Message> {
    let title = text(section.label())
        .size(20.0)
        .color(tokens.text_primary)
        .font(Font {
            weight: Weight::Semibold,
            ..ui_font()
        });
    let strip = container(title)
        .width(Length::Fill)
        .height(53.0)
        .align_y(alignment::Alignment::Center)
        .padding(Padding {
            top: 0.0,
            right: 18.0,
            bottom: 0.0,
            left: 18.0,
        });
    column![
        mouse_area(strip).on_press(Message::DragWindow),
        divider(tokens)
    ]
    .width(Length::Fill)
    .into()
}

fn vdivider(tokens: ThemeTokens) -> Element<'static, Message> {
    container(Space::new().width(1.0))
        .width(1.0)
        .height(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.border_window)),
            ..container::Style::default()
        })
        .into()
}

fn divider(tokens: ThemeTokens) -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.border_window)),
            ..container::Style::default()
        })
        .into()
}

fn toast(msg: &str, tokens: ThemeTokens) -> Element<'static, Message> {
    container(
        container(text(msg.to_string()).size(12.0).color(tokens.bg_window))
            .padding([8.0, 14.0])
            .style(move |_t| container::Style {
                background: Some(Background::Color(tokens.text_primary)),
                border: Border::default().rounded(999.0),
                ..container::Style::default()
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(alignment::Alignment::Center)
    .align_y(alignment::Alignment::End)
    .padding(Padding {
        top: 0.0,
        right: 0.0,
        bottom: 16.0,
        left: 0.0,
    })
    .into()
}
