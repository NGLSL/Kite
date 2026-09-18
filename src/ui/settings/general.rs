use iced::widget::{button, row, text};
use iced::{border, Color, Element};

use super::super::theme::{ThemeMode, ThemeTokens};
use super::super::{Message, State};
use super::widgets::{flow_card, flow_row, std_button, toggle};

pub(super) fn general_card<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    flow_card(
        vec![
            flow_row(
                "外观模式",
                "深色沉浸、清爽浅色，或跟随 Windows 自动切换".to_string(),
                theme_switcher(state.theme_mode, tokens),
                tokens,
            ),
            flow_row(
                "开机自动启动",
                "登录 Windows 后在后台待命".to_string(),
                toggle(state.autostart, Message::SetAutostart(!state.autostart), tokens),
                tokens,
            ),
            flow_row(
                "失焦时隐藏",
                "点击其它窗口后自动收起启动器".to_string(),
                toggle(
                    state.hide_on_blur,
                    Message::SetHideOnBlur(!state.hide_on_blur),
                    tokens,
                ),
                tokens,
            ),
            flow_row(
                "记录使用历史",
                "暂停后不再记录启动次数与查询偏好".to_string(),
                toggle(
                    state.history_recording,
                    Message::SetHistoryRecording(!state.history_recording),
                    tokens,
                ),
                tokens,
            ),
            flow_row(
                "查询与按键诊断日志",
                "关闭后键入与查询不再写日志，排查时再打开".to_string(),
                toggle(state.query_log, Message::SetQueryLog(!state.query_log), tokens),
                tokens,
            ),
            flow_row(
                "清空使用历史",
                "删除全部启动次数与查询配对，固定项不受影响".to_string(),
                std_button("清空", Message::ClearHistory, tokens),
                tokens,
            ),
        ],
        tokens,
    )
}

fn theme_switcher(current: ThemeMode, tokens: ThemeTokens) -> Element<'static, Message> {
    let opt = |label: &'static str, mode: ThemeMode| {
        let active = current == mode;
        button(text(label).size(12.0).color(if active {
            tokens.accent
        } else {
            tokens.text_primary
        }))
        .padding([5.0, 10.0])
        .on_press(Message::SetThemeMode(mode))
        .style(move |_t, status| button::Style {
            background: if active {
                Some(iced::Background::Color(Color {
                    a: 0.15,
                    ..tokens.accent
                }))
            } else if status == button::Status::Hovered {
                Some(iced::Background::Color(tokens.active_bg))
            } else {
                None
            },
            text_color: if active {
                tokens.accent
            } else {
                tokens.text_primary
            },
            border: iced::Border {
                color: if active {
                    Color {
                        a: 0.45,
                        ..tokens.accent
                    }
                } else {
                    tokens.border_window
                },
                width: 1.0,
                radius: border::radius(6.0),
            },
            ..button::Style::default()
        })
    };
    row![
        opt("深色", ThemeMode::Dark),
        opt("浅色", ThemeMode::Light),
        opt("跟随系统", ThemeMode::System),
    ]
    .spacing(6.0)
    .into()
}
