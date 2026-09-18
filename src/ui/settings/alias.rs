//! 别名管理。

use iced::widget::{button, column, container, row, text, text_input};
use iced::{alignment, border, color, Background, Border, Color, Element, Length};

use super::super::font::name_font;
use super::super::search_view::{BG_ELEVATED, BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED};
use super::super::{Message, State};

pub(super) fn alias_card(state: &State) -> Element<'_, Message> {
    let alias_input = text_input("别名，如 vsc", &state.alias_input)
        .on_input(Message::AliasInputChanged)
        .size(13.0)
        .padding([8.0, 10.0])
        .width(110.0)
        .style(input_style);
    let target_input = text_input("目标名称，如 visual", &state.alias_target_input)
        .on_input(Message::AliasTargetChanged)
        .size(13.0)
        .padding([8.0, 10.0])
        .width(Length::Fill)
        .style(input_style);
    let add_button = button(text("添加").size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(Message::AliasAdd)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(MARK)),
            text_color: color!(0xFF_FF_FF),
            border: Border {
                color: MARK,
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..button::Style::default()
        });
    let form = column![
        text("添加别名").size(13.0).color(TEXT).font(name_font()),
        row![
            column![text("别名").size(11.0).color(TEXT_MUTED), alias_input]
                .spacing(4.0)
                .width(110.0),
            column![text("目标应用").size(11.0).color(TEXT_MUTED), target_input]
                .spacing(4.0)
                .width(Length::Fill),
            column![text(" ").size(11.0), add_button].spacing(4.0),
        ]
        .spacing(8.0)
        .align_y(alignment::Alignment::End),
    ]
    .spacing(10.0);
    let form_card = elevated_card(form.into());

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
                            .width(Length::Fill),
                        text(c.item.target.clone()).size(11.0).color(TEXT_MUTED),
                    ]
                    .spacing(8.0)
                    .align_y(alignment::Alignment::Center),
                )
                .width(Length::Fill)
                .padding([6.0, 8.0])
                .on_press(Message::AliasPick(i))
                .style(move |_t, _s| button::Style {
                    background: if picked {
                        Some(Background::Color(Color { a: 0.12, ..MARK }))
                    } else {
                        None
                    },
                    text_color: if picked { MARK } else { TEXT },
                    border: Border::default().rounded(7.0),
                    ..button::Style::default()
                }),
            );
        }
        sections.push(elevated_card(
            column![text("选择目标应用").size(12.0).color(TEXT_MUTED), cand]
                .spacing(8.0)
                .into(),
        ));
    }

    if state.aliases.is_empty() {
        sections.push(elevated_card(
            column![
                text("已添加别名").size(13.0).color(TEXT).font(name_font()),
                text("暂无别名。输入别名并选择目标应用后添加。")
                    .size(12.0)
                    .color(TEXT_MUTED),
            ]
            .spacing(8.0)
            .into(),
        ));
    } else {
        let mut list =
            column![text("已添加别名").size(13.0).color(TEXT).font(name_font())].spacing(8.0);
        for a in &state.aliases {
            let alias = a.alias.clone();
            list = list.push(
                container(
                    row![
                        container(
                            text(a.alias.clone())
                                .size(13.0)
                                .color(MARK)
                                .font(name_font())
                        )
                        .padding([2.0, 8.0])
                        .style(|_t| container::Style {
                            background: Some(Background::Color(Color { a: 0.10, ..MARK })),
                            border: Border::default().rounded(6.0),
                            ..container::Style::default()
                        }),
                        text("→").size(12.0).color(TEXT_MUTED),
                        text(a.target_name.clone())
                            .size(13.0)
                            .color(TEXT_MUTED)
                            .width(Length::Fill),
                        button(text("删除").size(12.0).color(TEXT_MUTED))
                            .padding([4.0, 6.0])
                            .on_press(Message::AliasRemove(alias))
                            .style(|_t, _s| button::Style {
                                background: None,
                                text_color: TEXT_MUTED,
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
        sections.push(elevated_card(list.into()));
    }

    column(sections).spacing(12.0).width(Length::Fill).into()
}

fn input_style(_t: &iced::Theme, _s: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(BG_PANEL),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: border::radius(8.0),
        },
        icon: TEXT_MUTED,
        placeholder: TEXT_MUTED,
        value: TEXT,
        selection: Color { a: 0.25, ..MARK },
    }
}

fn elevated_card(content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .width(Length::Fill)
        .padding([12.0, 14.0])
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        })
        .into()
}
