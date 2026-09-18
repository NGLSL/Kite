//! Provider 插件结果面板。

use iced::widget::{column, container, row, text};
use iced::{border, color, Background, Border, Color, Element, Length, Padding};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::{divider, Message, State};
use crate::plugin::{PanelBlock, PanelData};
pub(super) fn panel_area<'a>(
    state: &'a State,
    panel: &PanelData,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    let mut col = column![].spacing(10.0);

    if let Some(act) = &state.provider_mode {
        let plugin_label = {
            let reg = state
                .plugin_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            reg.get(&act.plugin_id)
                .map(|p| p.manifest.plugin.name.clone())
                .unwrap_or_else(|| act.plugin_id.clone())
        };
        col = col.push(
            row![
                text(format!("{} · {}", plugin_label, act.provider_id))
                    .size(11.0)
                    .color(tokens.text_muted),
                text("Esc 退出").size(11.0).color(tokens.text_muted),
            ]
            .spacing(8.0)
            .width(Length::Fill),
        );
    }
    if let Some(err) = &state.plugin_flash {
        col = col.push(text(err.clone()).size(12.0).color(color!(0xDC_26_26)));
    }

    for block in &panel.blocks {
        match block {
            PanelBlock::Text { text: t, style } => {
                let c = match style.as_str() {
                    "secondary" | "muted" => tokens.text_muted,
                    "error" => color!(0xDC_26_26),
                    _ => tokens.text_primary,
                };
                col = col.push(text(t.clone()).size(14.0).color(c).width(Length::Fill));
            }
            PanelBlock::Value {
                label,
                value,
                selectable: _,
            } => {
                let mut card_col = column![].spacing(4.0);
                if let Some(l) = label {
                    card_col = card_col.push(text(l.clone()).size(11.5).color(tokens.text_muted));
                }
                card_col = card_col.push(
                    text(value.clone())
                        .size(36.0)
                        .font(iced::Font::MONOSPACE)
                        .color(tokens.text_primary)
                        .width(Length::Fill),
                );

                let card = container(card_col)
                    .width(Length::Fill)
                    .padding([14.0, 18.0])
                    .style(move |_t| container::Style {
                        background: Some(Background::Color(tokens.bg_elevated)),
                        border: Border {
                            color: tokens.border_subtle,
                            width: 1.0,
                            radius: border::radius(10.0),
                        },
                        ..container::Style::default()
                    });
                col = col.push(card);
            }
            PanelBlock::KeyValue { items } => {
                let mut kv = column![].spacing(6.0);
                for it in items {
                    kv = kv.push(
                        row![
                            text(it.key.clone())
                                .size(12.0)
                                .color(tokens.text_muted)
                                .width(Length::Fill),
                            text(it.value.clone())
                                .size(12.0)
                                .font(name_font())
                                .color(tokens.text_primary),
                        ]
                        .spacing(12.0),
                    );
                }
                let kv_card = container(kv)
                    .width(Length::Fill)
                    .padding([10.0, 14.0])
                    .style(move |_t| container::Style {
                        background: Some(Background::Color(tokens.bg_elevated)),
                        border: Border {
                            color: tokens.border_subtle,
                            width: 1.0,
                            radius: border::radius(8.0),
                        },
                        ..container::Style::default()
                    });
                col = col.push(kv_card);
            }
            PanelBlock::Notice { level, text: t } => {
                let (bg, fg) = match level.as_str() {
                    "error" => (
                        Color {
                            a: 0.15,
                            ..color!(0xDC_26_26)
                        },
                        color!(0xEF_44_44),
                    ),
                    "warning" => (
                        Color {
                            a: 0.15,
                            ..color!(0xF5_9E_0B)
                        },
                        color!(0xF5_9E_0B),
                    ),
                    _ => (
                        Color {
                            a: 0.12,
                            ..tokens.accent
                        },
                        tokens.accent,
                    ),
                };
                col = col.push(
                    container(text(t.clone()).size(13.0).color(fg).width(Length::Fill))
                        .width(Length::Fill)
                        .padding([8.0, 12.0])
                        .style(move |_t| container::Style {
                            background: Some(Background::Color(bg)),
                            border: Border::default().rounded(8.0),
                            ..container::Style::default()
                        }),
                );
            }
            PanelBlock::Divider => col = col.push(divider(tokens)),
        }
    }

    if let Some(action) = panel.actions.iter().find(|a| a.default) {
        col = col.push(
            text(format!("Enter · {}", action.label))
                .size(12.0)
                .color(tokens.text_muted),
        );
    }

    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 12.0,
            right: 18.0,
            bottom: 12.0,
            left: 18.0,
        })
        .into()
}
