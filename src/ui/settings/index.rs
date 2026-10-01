//! 索引与便携目录。

use iced::widget::{button, column, container, row, text, text_input};
use iced::{border, color, Background, Border, Color, Element, Length};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::{Message, State};
use super::widgets::{elevated_card, flow_card, flow_row, std_button, text_input_style};

pub(super) fn index_card<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let n = state.index.lock().map(|g| g.apps.len()).unwrap_or(0);
    let health = crate::app::index_health::load();
    let now = crate::storage::now_ts().max(0) as u64;
    let health_label = health.summary(now);
    let health_status = if crate::app::index_health::is_degraded(&health) {
        "需关注"
    } else {
        "正常"
    };
    let summary = flow_card(
        vec![
            flow_row(
                "重新扫描应用",
                "更新系统入口、软件注册信息和便携目录".to_string(),
                std_button("重新扫描", Message::Rescan, tokens),
                tokens,
            ),
            flow_row(
                "当前索引",
                "快速扫描 + AppsFolder 后台补齐".to_string(),
                text(format!("{n} 条"))
                    .size(13.0)
                    .color(tokens.text_primary)
                    .into(),
                tokens,
            ),
            flow_row(
                "索引健康",
                health_label,
                text(health_status)
                    .size(13.0)
                    .color(tokens.text_muted)
                    .into(),
                tokens,
            ),
        ],
        tokens,
    );

    let input = text_input(r"D:\Portable Apps", &state.portable_dir_input)
        .on_input(Message::PortableDirInputChanged)
        .on_submit(Message::AddPortableDir)
        .padding([7.0, 10.0])
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
        });
    let add = button(text("添加").size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(Message::AddPortableDir)
        .style(move |_t, _s| button::Style {
            background: Some(Background::Color(tokens.accent)),
            text_color: color!(0xFF_FF_FF),
            border: Border::default().rounded(8.0),
            ..button::Style::default()
        });
    let mut directories = column![
        text("便携软件目录")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font()),
        text("加入后扫描目录内的应用入口；修改会自动重建索引。")
            .size(12.0)
            .color(tokens.text_muted),
        row![input, add].spacing(8.0),
    ]
    .spacing(8.0);
    for (index, path) in state.portable_dirs.iter().enumerate() {
        directories = directories.push(
            row![
                text(path.clone())
                    .size(12.0)
                    .color(tokens.text_primary)
                    .width(Length::Fill),
                button(text("移除").size(12.0).color(tokens.text_muted))
                    .padding([4.0, 7.0])
                    .on_press(Message::RemovePortableDir(index))
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
            .align_y(iced::alignment::Alignment::Center),
        );
    }
    let directory_card = container(directories)
        .width(Length::Fill)
        .padding([12.0, 14.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_elevated)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(10.0),
            },
            ..container::Style::default()
        });

    let manual_path = text_input(r"C:\Apps\Kite.exe 或 C:\Apps\Kite.lnk", &state.manual_path_input)
        .on_input(Message::ManualPathInputChanged)
        .on_submit(Message::ManualAdd)
        .padding([7.0, 10.0])
        .width(Length::Fill)
        .style(text_input_style(tokens));
    let manual_name = text_input("显示名称", &state.manual_name_input)
        .on_input(Message::ManualNameInputChanged)
        .on_submit(Message::ManualAdd)
        .padding([7.0, 10.0])
        .width(180.0)
        .style(text_input_style(tokens));
    let manual_add = std_button("登记应用", Message::ManualAdd, tokens);
    let manual_form = elevated_card(
        column![
            text("手动添加应用")
                .size(13.0)
                .color(tokens.text_primary)
                .font(name_font()),
            text("支持已存在的绝对路径 exe 和可解析的 lnk；登记不会移动或复制目标文件。")
                .size(12.0)
                .color(tokens.text_muted),
            row![manual_path, manual_name, manual_add]
                .spacing(8.0)
                .align_y(iced::alignment::Alignment::Center),
        ]
        .spacing(8.0)
        .into(),
        tokens,
    );

    let mut manual_body = column![
        text("已登记应用")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font()),
    ]
    .spacing(7.0);
    if state.manual_apps.is_empty() {
        manual_body = manual_body.push(
            text("暂无手动登记。搜索绝对路径时也可以从结果面板直接添加。")
                .size(12.0)
                .color(tokens.text_muted),
        );
    } else {
        for app in &state.manual_apps {
            let id = app.id;
            let edit = state
                .manual_name_edits
                .get(&id)
                .cloned()
                .unwrap_or_else(|| app.display_name.clone());
            let status = state
                .manual_status
                .get(&id)
                .map(String::as_str)
                .unwrap_or("状态未知");
            let status_color = if status == "可用" {
                tokens.accent
            } else {
                tokens.text_muted
            };
            manual_body = manual_body.push(
                row![
                    column![
                        text(app.path.clone())
                            .size(12.0)
                            .color(tokens.text_primary),
                        text(status)
                            .size(10.5)
                            .color(status_color),
                    ]
                    .spacing(2.0)
                    .width(Length::Fill),
                    text_input("名称", &edit)
                        .on_input(move |value| Message::ManualRenameInputChanged(id, value))
                        .padding([5.0, 7.0])
                        .width(150.0)
                        .style(text_input_style(tokens)),
                    button(text("保存").size(12.0).color(tokens.text_muted))
                        .padding([4.0, 7.0])
                        .on_press(Message::ManualRename(id)),
                    button(text("移除").size(12.0).color(tokens.text_muted))
                        .padding([4.0, 7.0])
                        .on_press(Message::ManualRemove(id)),
                ]
                .spacing(7.0)
                .align_y(iced::alignment::Alignment::Center),
            );
        }
    }
    let manual_list = elevated_card(manual_body.into(), tokens);

    let mut hidden_body = column![
        text("已隐藏入口")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font()),
        text("隐藏只影响应用建议；原有 Alias、固定和历史在恢复后继续有效。")
            .size(12.0)
            .color(tokens.text_muted),
    ]
    .spacing(7.0);
    if state.hidden_items.is_empty() {
        hidden_body = hidden_body.push(
            text("暂无隐藏入口。")
                .size(12.0)
                .color(tokens.text_muted),
        );
    } else {
        for hidden in &state.hidden_items {
            let item_id = hidden.item_id.clone();
            let args = hidden
                .args
                .as_deref()
                .filter(|args| !args.trim().is_empty())
                .map(|args| format!(" · 参数 {args}"))
                .unwrap_or_default();
            hidden_body = hidden_body.push(
                row![
                    column![
                        text(hidden.display_name.clone())
                            .size(12.0)
                            .color(tokens.text_primary),
                        text(format!("{}{}", hidden.target, args))
                            .size(10.5)
                            .color(tokens.text_muted),
                    ]
                    .spacing(2.0)
                    .width(Length::Fill),
                    button(text("恢复").size(12.0).color(tokens.text_muted))
                        .padding([4.0, 8.0])
                        .on_press(Message::RestoreHidden(item_id)),
                ]
                .spacing(8.0)
                .align_y(iced::alignment::Alignment::Center),
            );
        }
    }
    let hidden_list = elevated_card(hidden_body.into(), tokens);

    column![summary, manual_form, manual_list, hidden_list, directory_card]
        .spacing(12.0)
        .width(Length::Fill)
        .into()
}
