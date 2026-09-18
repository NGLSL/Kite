//! 别名管理。

use iced::widget::{button, column, container, row, text, text_input};
use iced::{alignment, border, color, Background, Border, Color, Element, Length};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::{Message, State};
use super::widgets::{elevated_card, text_input_style};

pub(super) fn alias_card<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let alias_input = text_input("别名，如 vsc", &state.alias_input)
        .on_input(Message::AliasInputChanged)
        .size(13.0)
        .padding([8.0, 10.0])
        .width(110.0)
        .style(text_input_style(tokens));
    let target_input = text_input("目标名称，如 visual", &state.alias_target_input)
        .on_input(Message::AliasTargetChanged)
        .size(13.0)
        .padding([8.0, 10.0])
        .width(Length::Fill)
        .style(text_input_style(tokens));
    let add_button = button(text("添加").size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(Message::AliasAdd)
        .style(move |_t, _s| button::Style {
            background: Some(Background::Color(tokens.accent)),
            text_color: color!(0xFF_FF_FF),
            border: Border {
                color: tokens.accent,
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..button::Style::default()
        });
    let form = column![
        text("添加别名")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font()),
        row![
            column![
                text("别名").size(11.0).color(tokens.text_muted),
                alias_input
            ]
            .spacing(4.0)
            .width(110.0),
            column![
                text("目标应用").size(11.0).color(tokens.text_muted),
                target_input
            ]
            .spacing(4.0)
            .width(Length::Fill),
            column![text(" ").size(11.0), add_button].spacing(4.0),
        ]
        .spacing(8.0)
        .align_y(alignment::Alignment::End),
    ]
    .spacing(10.0);
    let form_card = elevated_card(form.into(), tokens);

    let mut sections: Vec<Element<'_, Message>> = vec![form_card];

    if !state.alias_candidates.is_empty() {
        let mut cand = column![].spacing(2.0);
        for (i, c) in state.alias_candidates.iter().enumerate() {
            let picked = state
                .alias_pick
                .as_ref()
                .map(|p| p.target_id.as_deref() == Some(c.item.id.as_str()))
                .unwrap_or(false);
            cand = cand.push(
                button(
                    row![
                        text(c.item.display_name.clone())
                            .size(13.0)
                            .color(if picked {
                                tokens.accent
                            } else {
                                tokens.text_primary
                            })
                            .width(Length::Fill),
                        text(c.item.target.clone())
                            .size(11.0)
                            .color(tokens.text_muted),
                    ]
                    .spacing(8.0)
                    .align_y(alignment::Alignment::Center),
                )
                .width(Length::Fill)
                .padding([6.0, 8.0])
                .on_press(Message::AliasPick(i))
                .style(move |_t, status| button::Style {
                    background: if picked {
                        Some(Background::Color(Color {
                            a: 0.12,
                            ..tokens.accent
                        }))
                    } else if status == button::Status::Hovered {
                        Some(Background::Color(tokens.active_bg))
                    } else {
                        None
                    },
                    text_color: if picked {
                        tokens.accent
                    } else {
                        tokens.text_primary
                    },
                    border: Border::default().rounded(7.0),
                    ..button::Style::default()
                }),
            );
        }
        sections.push(elevated_card(
            column![
                text("选择目标应用").size(12.0).color(tokens.text_muted),
                cand
            ]
            .spacing(8.0)
            .into(),
            tokens,
        ));
    }

    if state.aliases.is_empty() {
        sections.push(elevated_card(
            column![
                text("已添加别名")
                    .size(13.0)
                    .color(tokens.text_primary)
                    .font(name_font()),
                text("暂无别名。输入别名并选择目标应用后添加。")
                    .size(12.0)
                    .color(tokens.text_muted),
            ]
            .spacing(8.0)
            .into(),
            tokens,
        ));
    } else {
        let mut list = column![text("已添加别名")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font())]
        .spacing(8.0);
        for a in &state.aliases {
            let alias = a.alias.clone();
            list = list.push(
                container(
                    row![
                        container(
                            text(a.alias.clone())
                                .size(13.0)
                                .color(tokens.accent)
                                .font(name_font())
                        )
                        .padding([2.0, 8.0])
                        .style(move |_t| container::Style {
                            background: Some(Background::Color(Color {
                                a: 0.12,
                                ..tokens.accent
                            })),
                            border: Border::default().rounded(6.0),
                            ..container::Style::default()
                        }),
                        text("→").size(12.0).color(tokens.text_muted),
                        text(a.target_name.clone())
                            .size(13.0)
                            .color(tokens.text_muted)
                            .width(Length::Fill),
                        button(text("删除").size(12.0).color(tokens.text_muted))
                            .padding([4.0, 6.0])
                            .on_press(Message::AliasRemove(alias))
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
                    .spacing(8.0)
                    .align_y(alignment::Alignment::Center),
                )
                .width(Length::Fill)
                .padding([8.0, 14.0]),
            );
        }
        sections.push(elevated_card(list.into(), tokens));
    }

    column(sections).spacing(12.0).width(Length::Fill).into()
}
