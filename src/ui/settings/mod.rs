//! 设置页：壳层（导航/标题/toast）+ 分区卡片 + 共用控件。

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
use super::search_view::{BG_ELEVATED, BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED, WINDOW_BORDER};
use super::{Message, Section, State};

/// 设置页入口：左导航 + 右内容区 + 底部 toast。
pub fn settings_view(state: &State) -> Element<'_, Message> {
    let nav = nav_panel(state);
    let main = column![
        header(state.settings_section),
        scrollable(
            container(body(state))
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
        .style(super::search_view::results_scroll_style)
        .width(Length::Fill)
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let root = container(row![nav, vdivider(), main])
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
        });

    match &state.flash {
        Some(msg) => {
            let root: Element<'_, Message> = root.into();
            iced::widget::stack![root, toast(msg)].into()
        }
        None => root.into(),
    }
}

fn body(state: &State) -> Element<'_, Message> {
    match state.settings_section {
        Section::General => general::general_card(state),
        Section::SearchEngine => search_engine::search_engine_card(state),
        Section::Hotkey => hotkey::hotkey_card(state),
        Section::Alias => alias::alias_card(state),
        Section::Index => index::index_card(state),
        Section::Plugins => plugins::plugins_card(state),
        Section::About => about::about_card(state),
    }
}

fn nav_panel(state: &State) -> Element<'_, Message> {
    let logo = container(text("K").size(15.0).color(color!(0xFF_FF_FF)).font(Font {
        weight: Weight::Bold,
        ..ui_font()
    }))
    .width(32.0)
    .height(32.0)
    .align_x(alignment::Alignment::Center)
    .align_y(alignment::Alignment::Center)
    .style(|_t| container::Style {
        background: Some(Background::Color(MARK)),
        border: Border::default().rounded(8.0),
        ..container::Style::default()
    });

    let brand = row![
        logo,
        column![
            text("Kite")
                .size(13.0)
                .font(super::font::name_font())
                .color(TEXT),
            text("设置").size(11.0).color(TEXT_MUTED),
        ]
        .spacing(1.0)
        .width(Length::Fill),
        button(text("×").size(16.0).color(TEXT_MUTED))
            .padding([4.0, 8.0])
            .on_press(Message::CloseSettings)
            .style(|_t, _s| button::Style {
                background: None,
                text_color: TEXT_MUTED,
                border: Border::default().rounded(8.0),
                ..button::Style::default()
            }),
    ]
    .spacing(10.0)
    .align_y(alignment::Alignment::Center);

    let mut nav_list = column![].spacing(2.0).padding(8.0);
    for section in Section::ALL {
        let on = state.settings_section == section;
        nav_list = nav_list.push(
            button(
                text(section.label())
                    .size(13.0)
                    .color(if on { MARK } else { TEXT_MUTED })
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
            .style(move |_t, _s| button::Style {
                background: if on {
                    Some(Background::Color(Color { a: 0.12, ..MARK }))
                } else {
                    None
                },
                text_color: if on { MARK } else { TEXT_MUTED },
                border: Border::default().rounded(8.0),
                ..button::Style::default()
            }),
        );
    }

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
        divider(),
        nav_list,
    ])
    .width(168.0)
    .height(Length::Fill)
    .style(|_t| container::Style {
        background: Some(Background::Color(BG_ELEVATED)),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: border::radius(0.0),
        },
        ..container::Style::default()
    })
    .into()
}

fn header(section: Section) -> Element<'static, Message> {
    let title = text(section.label()).size(20.0).color(TEXT).font(Font {
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
    column![mouse_area(strip).on_press(Message::DragWindow), divider()]
        .width(Length::Fill)
        .into()
}

fn vdivider() -> Element<'static, Message> {
    container(Space::new().width(1.0))
        .width(1.0)
        .height(Length::Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(BORDER)),
            ..container::Style::default()
        })
        .into()
}

fn divider() -> Element<'static, Message> {
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(|_t| container::Style {
            background: Some(Background::Color(BORDER)),
            ..container::Style::default()
        })
        .into()
}

fn toast(msg: &str) -> Element<'static, Message> {
    container(
        container(text(msg.to_string()).size(12.0).color(BG_PANEL))
            .padding([8.0, 14.0])
            .style(|_t| container::Style {
                background: Some(Background::Color(Color { a: 0.92, ..TEXT })),
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
