//! 搜索页底部提示、Toast 与右键菜单浮层。

use iced::widget::{button, column, container, row, space::Space, text, text_input};
use iced::{alignment, border, Background, Border, Color, Element, Length, Padding};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::{actions, NavigationMode};
use super::{Message, State};
pub(super) fn footer_bar(state: &State, tokens: ThemeTokens) -> Element<'static, Message> {
    let hotkey_label = if state.hotkey_label.is_empty() {
        "Alt + Space".to_string()
    } else {
        state.hotkey_label.clone()
    };

    let mut bar = row![].spacing(12.0).align_y(alignment::Alignment::Center);

    if state.provider_mode.is_some() {
        bar = bar
            .push(keycap_hint("↵", "复制结果", tokens))
            .push(keycap_hint("Ctrl+↵", "复制并继续", tokens))
            .push(keycap_hint("⌫", "退出模式", tokens));
    } else if state.query.trim().is_empty() {
        if state.navigation_mode == NavigationMode::Results {
            bar = bar
                .push(keycap_hint("↑↓←→", "选择", tokens))
                .push(keycap_hint("↵", "打开", tokens))
                .push(keycap_hint("Esc", "返回输入", tokens));
        } else {
            bar = bar
                .push(keycap_hint("↑↓", "选择", tokens))
                .push(keycap_hint("↵", "打开", tokens))
                .push(keycap_hint("Esc", "隐藏", tokens));
        }
    } else {
        if state.navigation_mode == NavigationMode::Results {
            bar = bar
                .push(keycap_hint("↑↓←→", "选择", tokens))
                .push(keycap_hint("↵", "打开应用", tokens))
                .push(keycap_hint("Ctrl+↵", "所在目录", tokens))
                .push(keycap_hint("Esc", "返回输入", tokens));
        } else {
            bar = bar
                .push(keycap_hint("↑↓", "选择", tokens))
                .push(keycap_hint("↵", "打开应用", tokens))
                .push(keycap_hint("Ctrl+↵", "所在目录", tokens))
                .push(keycap_hint("Esc", "关闭", tokens));
        }
    }

    bar = bar.push(Space::new().width(Length::Fill));
    bar = bar.push(text(hotkey_label).size(11.0).color(tokens.text_muted));

    let bg = tokens.bg_elevated;
    container(bar)
        .width(Length::Fill)
        .padding([8.0, 16.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(bg)),
            ..container::Style::default()
        })
        .into()
}

fn keycap_hint(
    k: &'static str,
    label: &'static str,
    tokens: ThemeTokens,
) -> Element<'static, Message> {
    row![
        container(
            text(k)
                .size(10.0)
                .font(name_font())
                .color(tokens.keycap_text)
        )
        .padding([1.5, 5.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.keycap_bg)),
            border: Border {
                color: tokens.keycap_border,
                width: 1.0,
                radius: border::radius(4.0),
            },
            ..container::Style::default()
        }),
        text(label).size(11.0).color(tokens.text_muted)
    ]
    .spacing(5.0)
    .align_y(alignment::Alignment::Center)
    .into()
}

pub(super) fn toast(msg: &str, tokens: ThemeTokens) -> Element<'static, Message> {
    container(
        container(
            row![
                text("✓").size(12.0).color(tokens.accent),
                text(msg.to_string()).size(12.0).color(tokens.text_primary),
            ]
            .spacing(6.0)
            .align_y(alignment::Alignment::Center),
        )
        .padding([7.0, 14.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(999.0),
            },
            shadow: iced::Shadow {
                color: Color {
                    a: 0.25,
                    ..Color::BLACK
                },
                offset: iced::Vector::new(0.0, 2.0),
                blur_radius: 8.0,
            },
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
        bottom: 40.0,
        left: 0.0,
    })
    .into()
}

/// 右键菜单 overlay
pub(super) fn menu_overlay<'a>(
    state: &'a State,
    item: &'a crate::model::AppItem,
    x: f32,
    y: f32,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    if state.action_alias_item.is_some() {
        return action_alias_overlay(state, x, y, tokens);
    }
    let entries = actions::menu_entries(state, item);

    let mut col = column![].width(Length::Fill);
    for (index, (label, action)) in entries.into_iter().enumerate() {
        let selected = state.menu_selected == index;
        col = col.push(
            button(text(label).size(13.0))
                .width(Length::Fill)
                .padding([7.0, 10.0])
                .on_press(Message::MenuAction(item.clone(), action))
                .style(move |_t, status| button::Style {
                    background: if selected || status == button::Status::Hovered {
                        Some(Background::Color(tokens.active_bg))
                    } else {
                        None
                    },
                    text_color: if status == button::Status::Hovered {
                        tokens.accent
                    } else {
                        tokens.text_primary
                    },
                    border: Border::default().rounded(7.0),
                    ..button::Style::default()
                }),
        );
    }

    let menu_box = container(col)
        .width(240.0)
        .padding(4.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        });

    container(menu_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Alignment::Start)
        .align_y(alignment::Alignment::Start)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

fn action_alias_overlay<'a>(
    state: &'a State,
    x: f32,
    y: f32,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    let Some(item) = state.action_alias_item.as_ref() else {
        return Space::new().width(Length::Fill).height(Length::Fill).into();
    };
    let input = text_input("输入 Alias，如 vs", &state.action_alias_input)
        .id(super::action_alias_input_id())
        .on_input(Message::ActionAliasInputChanged)
        .on_submit(Message::ActionAliasSave)
        .padding([7.0, 9.0])
        .width(Length::Fill)
        .style(move |_t, _s| iced::widget::text_input::Style {
            background: Background::Color(tokens.bg_input),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(7.0),
            },
            icon: tokens.text_muted,
            placeholder: tokens.text_muted,
            value: tokens.text_primary,
            selection: Color { a: 0.25, ..tokens.accent },
        });
    let mut content = column![
        text("设置 Alias").size(13.0).color(tokens.text_primary),
        text(item.display_name.clone()).size(11.0).color(tokens.text_muted),
        input,
    ]
    .spacing(7.0)
    .width(220.0);
    if let Some(conflict) = &state.action_alias_conflict {
        content = content.push(
            container(
                column![
                    text("该 Alias 已指向").size(11.0).color(tokens.text_muted),
                    text(conflict.target_name.clone()).size(12.0).color(tokens.text_primary),
                    text("确认后将替换原映射").size(11.0).color(tokens.text_muted),
                    row![
                        button(text("替换").size(12.0))
                            .padding([5.0, 9.0])
                            .on_press(Message::ActionAliasReplace),
                        button(text("取消").size(12.0))
                            .padding([5.0, 9.0])
                            .on_press(Message::ActionAliasCancel),
                    ]
                    .spacing(6.0),
                ]
                .spacing(4.0),
            )
            .padding(7.0)
            .style(move |_t| container::Style {
                background: Some(Background::Color(Color { a: 0.09, ..tokens.accent })),
                border: Border {
                    color: tokens.active_border,
                    width: 1.0,
                    radius: border::radius(7.0),
                },
                ..container::Style::default()
            }),
        );
    } else {
        content = content.push(
            row![
                button(text("保存").size(12.0))
                    .padding([5.0, 9.0])
                    .on_press(Message::ActionAliasSave),
                button(text("取消").size(12.0))
                    .padding([5.0, 9.0])
                    .on_press(Message::ActionAliasCancel),
            ]
            .spacing(6.0),
        );
    }
    let menu_box = container(content)
        .padding(10.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        });
    container(menu_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Alignment::Start)
        .align_y(alignment::Alignment::Start)
        .padding(Padding {
            top: y,
            left: x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}
