//! 搜索页底部提示、Toast 与右键菜单浮层。

use iced::widget::{button, column, container, row, space::Space, text};
use iced::{alignment, border, Background, Border, Color, Element, Length, Padding};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::{MenuAction, NavigationMode};
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
    let target_is_fs =
        std::path::Path::new(&item.target).is_file() || std::path::Path::new(&item.target).is_dir();
    let pinned = state.pinned.contains(&item.id);

    let is_shell = item
        .target
        .trim()
        .to_ascii_lowercase()
        .starts_with("shell:appsfolder");
    let mut entries: Vec<(&'static str, MenuAction)> = Vec::new();
    if target_is_fs {
        entries.push(("打开所在文件夹", MenuAction::OpenFolder));
        entries.push(("复制路径", MenuAction::CopyPath));
    } else if is_shell {
        entries.push(("复制 AUMID", MenuAction::CopyTarget));
    } else if !item.target.trim().is_empty() {
        entries.push(("复制 target", MenuAction::CopyTarget));
    }
    entries.push(("复制名称", MenuAction::CopyName));
    if item.source != "everything-status" && item.source != "direct-path" {
        entries.push((
            if pinned { "取消固定" } else { "固定" },
            MenuAction::TogglePin,
        ));
        let demoted = state
            .history
            .as_ref()
            .map(|db| db.is_demoted(&item.id))
            .unwrap_or(false);
        entries.push((
            if demoted {
                "恢复优先级"
            } else {
                "降低此结果优先级"
            },
            if demoted {
                MenuAction::Undemote
            } else {
                MenuAction::Demote
            },
        ));
    }

    let mut col = column![].width(Length::Fill);
    for (label, action) in entries {
        col = col.push(
            button(text(label).size(13.0))
                .width(Length::Fill)
                .padding([7.0, 10.0])
                .on_press(Message::MenuAction(item.clone(), action))
                .style(move |_t, status| button::Style {
                    background: if status == button::Status::Hovered {
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
        .width(180.0)
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
