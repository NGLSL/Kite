//! 设置页（对齐 SettingsPanel.tsx + settings.css）：左导航 168px + 右 Flow 卡片。
//! 布局 token 复用 ui.rs 的设计常量。

use iced::font::Weight;
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space::Space, text, text_input,
};
use iced::{alignment, border, color, Background, Border, Color, Element, Font, Length, Padding};

use super::font::{name_font, ui_font};
use super::search_view::{BG_ELEVATED, BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED, WINDOW_BORDER};
use super::{Message, Section, State};

const HOTKEY_PRESETS: [&str; 5] = [
    "Alt+Space",
    "Ctrl+Alt+Space",
    "Ctrl+Shift+K",
    "Alt+K",
    "Ctrl+Alt+A",
];

pub fn settings_view(state: &State) -> Element<'_, Message> {
    let nav = nav_panel(state);
    let main = column![
        header(state.settings_section),
        scrollable(body(state))
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

    // 底部提示条（css settings-toast）
    match &state.flash {
        Some(msg) => {
            let root: Element<'_, Message> = root.into();
            iced::widget::stack![root, toast(msg)].into()
        }
        None => root.into(),
    }
}

/// 左导航（css .settings-nav）：品牌块 + 分区列表。
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
            text("Kite").size(13.0).font(name_font()).color(TEXT),
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

    // 品牌区固定高 53（与右侧标题栏一致），两条分隔线精确对齐在同一 y
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
    // 标题栏固定高 53（与左侧品牌区一致，分隔线对齐），整条可拖拽
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

fn body(state: &State) -> Element<'_, Message> {
    let content: Element<'_, Message> = match state.settings_section {
        Section::General => general_card(state),
        Section::Hotkey => hotkey_card(state),
        Section::Alias => alias_card(state),
        Section::Index => index_card(state),
        Section::About => about_card(state),
    };
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 12.0,
            right: 14.0,
            bottom: 16.0,
            left: 14.0,
        })
        .into()
}

/// 一行 Flow 设置（css .flow-row）：标题+说明 在左，控件在右。
fn flow_row(
    title: &'static str,
    hint: String,
    control: Element<'static, Message>,
) -> Element<'static, Message> {
    container(
        row![
            column![
                text(title).size(13.5).color(TEXT).font(name_font()),
                text(hint).size(12.0).color(TEXT_MUTED)
            ]
            .spacing(2.0)
            .width(Length::Fill),
            control,
        ]
        .spacing(12.0)
        .align_y(alignment::Alignment::Center),
    )
    .width(Length::Fill)
    .padding([12.0, 14.0])
    .style(|_t| container::Style {
        border: Border {
            color: Color { a: 0.8, ..BORDER },
            width: 0.0,
            radius: border::radius(0.0),
        },
        ..container::Style::default()
    })
    .into()
}

/// 卡片（css .flow-card）：bg-elevated + 边框 + radius 10，行间 1px 分隔线。
fn flow_card(rows: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    let mut col = column![].width(Length::Fill);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            let d: Element<'static, Message> = container(Space::new().height(1.0))
                .width(Length::Fill)
                .height(1.0)
                .style(|_t| container::Style {
                    background: Some(Background::Color(Color { a: 0.8, ..BORDER })),
                    ..container::Style::default()
                })
                .into();
            col = col.push(d);
        }
        col = col.push(r);
    }
    container(col)
        .width(Length::Fill)
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

/// 开关（css .switch）：40×22 胶囊 + 白色滑块。
fn toggle(checked: bool, on_press: Message) -> Element<'static, Message> {
    button(
        container(
            container(Space::new().width(14.0).height(14.0)).style(|_t| container::Style {
                background: Some(Background::Color(color!(0xFF_FF_FF))),
                border: Border::default().rounded(999.0),
                ..container::Style::default()
            }),
        )
        .width(40.0)
        .height(22.0)
        .padding(2.0)
        .align_x(if checked {
            alignment::Alignment::End
        } else {
            alignment::Alignment::Start
        })
        .align_y(alignment::Alignment::Center),
    )
    .on_press(on_press)
    .style(move |_t, _s| button::Style {
        background: Some(Background::Color(if checked {
            MARK
        } else {
            Color { a: 0.18, ..TEXT }
        })),
        border: Border::default().rounded(999.0),
        ..button::Style::default()
    })
    .into()
}

/// 普通按钮（css .btn）。
fn std_button(label: &'static str, on_press: Message) -> Element<'static, Message> {
    button(text(label).size(13.0).color(TEXT))
        .padding([7.0, 12.0])
        .on_press(on_press)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(BG_PANEL)),
            text_color: TEXT,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..button::Style::default()
        })
        .into()
}

fn general_card(state: &State) -> Element<'_, Message> {
    flow_card(vec![
        flow_row(
            "开机自动启动",
            "登录 Windows 后在后台待命".to_string(),
            toggle(state.autostart, Message::SetAutostart(!state.autostart)),
        ),
        flow_row(
            "失焦时隐藏",
            "点击其它窗口后自动收起启动器".to_string(),
            toggle(
                state.hide_on_blur,
                Message::SetHideOnBlur(!state.hide_on_blur),
            ),
        ),
        flow_row(
            "记录使用历史",
            "暂停后不再记录启动次数与查询偏好".to_string(),
            toggle(
                state.history_recording,
                Message::SetHistoryRecording(!state.history_recording),
            ),
        ),
        flow_row(
            "查询与按键诊断日志",
            "关闭后键入与查询不再写日志，排查时再打开".to_string(),
            toggle(state.query_log, Message::SetQueryLog(!state.query_log)),
        ),
        flow_row(
            "清空使用历史",
            "删除全部启动次数与查询配对，固定项不受影响".to_string(),
            std_button("清空", Message::ClearHistory),
        ),
    ])
}

fn hotkey_card(state: &State) -> Element<'_, Message> {
    let recording = state.hotkey_recording;
    let chip = container(
        text(state.hotkey_label.clone())
            .size(13.0)
            .color(TEXT)
            .font(Font {
                weight: Weight::Semibold,
                ..ui_font()
            }),
    )
    .padding([5.0, 10.0])
    .style(|_t| container::Style {
        background: Some(Background::Color(BG_PANEL)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: border::radius(6.0),
        },
        ..container::Style::default()
    });

    let change: Element<'_, Message> = button(
        text(if recording {
            "按下组合键…"
        } else {
            "更改"
        })
        .size(13.0)
        .color(if recording { color!(0xFF_FF_FF) } else { TEXT }),
    )
    .padding([7.0, 12.0])
    .on_press(Message::StartHotkeyRecord)
    .style(move |_t, _s| button::Style {
        background: if recording {
            Some(Background::Color(MARK))
        } else {
            Some(Background::Color(BG_PANEL))
        },
        text_color: if recording { color!(0xFF_FF_FF) } else { TEXT },
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: border::radius(8.0),
        },
        ..button::Style::default()
    })
    .into();

    let mut presets = row![].spacing(6.0);
    for preset in HOTKEY_PRESETS {
        let on = state.hotkey == preset;
        presets = presets.push(
            button(
                text(preset)
                    .size(12.0)
                    .color(if on { MARK } else { TEXT_MUTED }),
            )
            .padding([5.0, 10.0])
            .on_press(Message::ApplyHotkey(preset.to_string()))
            .style(move |_t, _s| button::Style {
                background: if on {
                    Some(Background::Color(Color { a: 0.10, ..MARK }))
                } else {
                    Some(Background::Color(BG_PANEL))
                },
                text_color: if on { MARK } else { TEXT_MUTED },
                border: Border {
                    color: if on {
                        Color { a: 0.45, ..MARK }
                    } else {
                        BORDER
                    },
                    width: 1.0,
                    radius: border::radius(999.0),
                },
                ..button::Style::default()
            }),
        );
    }

    flow_card(vec![
        flow_row(
            "全局快捷键",
            "按下组合键后立即生效".to_string(),
            row![chip, change]
                .spacing(8.0)
                .align_y(alignment::Alignment::Center)
                .into(),
        ),
        container(column![text("常用预设").size(12.0).color(TEXT_MUTED), presets,].spacing(8.0))
            .width(Length::Fill)
            .padding(Padding {
                top: 12.0,
                right: 14.0,
                bottom: 14.0,
                left: 14.0,
            })
            .into(),
    ])
}

fn alias_card(state: &State) -> Element<'_, Message> {
    // 添加表单：把字段说明放在输入框上方，避免两个输入框在窄窗口中挤成一条难读的长条。
    let alias_input = text_input("别名，如 vsc", &state.alias_input)
        .on_input(Message::AliasInputChanged)
        .size(13.0)
        .padding([8.0, 10.0])
        .width(110.0)
        .style(|_t, _s| text_input::Style {
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
        });
    let target_input = text_input("目标名称，如 visual", &state.alias_target_input)
        .on_input(Message::AliasTargetChanged)
        .size(13.0)
        .padding([8.0, 10.0])
        .width(Length::Fill)
        .style(|_t, _s| text_input::Style {
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
        });
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
    let form_card = container(form)
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
        });

    let mut sections: Vec<Element<'_, Message>> = vec![form_card.into()];

    // 候选（目标搜索结果，点击选定）。
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
        sections.push(
            container(
                column![text("选择目标应用").size(12.0).color(TEXT_MUTED), cand].spacing(8.0),
            )
            .width(Length::Fill)
            .padding([10.0, 14.0])
            .style(|_t| container::Style {
                background: Some(Background::Color(BG_ELEVATED)),
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: border::radius(10.0),
                },
                ..container::Style::default()
            })
            .into(),
        );
    }

    // 现有别名列表：独立成卡片，和候选结果保持明确的视觉层级。
    if state.aliases.is_empty() {
        sections.push(
            container(
                column![
                    text("已添加别名").size(13.0).color(TEXT).font(name_font()),
                    text("暂无别名。输入别名并选择目标应用后添加。")
                        .size(12.0)
                        .color(TEXT_MUTED),
                ]
                .spacing(8.0),
            )
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
            .into(),
        );
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
        sections.push(
            container(list)
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
                .into(),
        );
    }

    column(sections).spacing(12.0).width(Length::Fill).into()
}

fn index_card(state: &State) -> Element<'_, Message> {
    let n = state.index.lock().map(|g| g.apps.len()).unwrap_or(0);
    let summary = flow_card(vec![
        flow_row(
            "重新扫描应用",
            "更新系统入口、软件注册信息和便携目录".to_string(),
            std_button("重新扫描", Message::Rescan),
        ),
        flow_row(
            "当前索引",
            "快速扫描 + AppsFolder 后台补齐".to_string(),
            text(format!("{n} 条")).size(13.0).color(TEXT).into(),
        ),
    ]);

    let input = text_input(r"D:\Portable Apps", &state.portable_dir_input)
        .on_input(Message::PortableDirInputChanged)
        .on_submit(Message::AddPortableDir)
        .padding([7.0, 10.0])
        .width(Length::Fill)
        .style(|_t, _s| text_input::Style {
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
        });
    let add = button(text("添加").size(13.0).color(color!(0xFF_FF_FF)))
        .padding([7.0, 12.0])
        .on_press(Message::AddPortableDir)
        .style(|_t, _s| button::Style {
            background: Some(Background::Color(MARK)),
            text_color: color!(0xFF_FF_FF),
            border: Border::default().rounded(8.0),
            ..button::Style::default()
        });
    let mut directories = column![
        text("便携软件目录")
            .size(13.0)
            .color(TEXT)
            .font(name_font()),
        text("加入后扫描目录内的应用入口；修改会自动重建索引。")
            .size(12.0)
            .color(TEXT_MUTED),
        row![input, add].spacing(8.0),
    ]
    .spacing(8.0);
    for (index, path) in state.portable_dirs.iter().enumerate() {
        directories = directories.push(
            row![
                text(path.clone())
                    .size(12.0)
                    .color(TEXT)
                    .width(Length::Fill),
                button(text("移除").size(12.0).color(TEXT_MUTED))
                    .padding([4.0, 7.0])
                    .on_press(Message::RemovePortableDir(index))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: TEXT_MUTED,
                        border: Border::default().rounded(6.0),
                        ..button::Style::default()
                    }),
            ]
            .spacing(8.0)
            .align_y(alignment::Alignment::Center),
        );
    }
    let directory_card = container(directories)
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
        });

    column![summary, directory_card]
        .spacing(12.0)
        .width(Length::Fill)
        .into()
}

fn about_card(state: &State) -> Element<'_, Message> {
    // 版本、更新与项目链接都在关于页，手动安装入口始终可用。
    let hint = match &state.update_status {
        None => "对比 GitHub 最新发布版本".to_string(),
        Some(Ok(latest)) if latest == "latest" => "已是最新版本".to_string(),
        Some(Ok(latest)) => format!("发现新版本 {latest}"),
        Some(Err(e)) => format!("更新失败：{e}"),
    };
    let available = matches!(
        &state.update_status,
        Some(Ok(latest)) if latest != "latest"
    );

    let control: Element<'static, Message> = if available {
        // 下载并启动安装器；无资产时退回发布页。
        let label = if state.update_checking {
            "下载中…"
        } else if state.update_asset.is_some() {
            "下载并安装"
        } else {
            "查看发布页"
        };
        let button = button(text(label).size(13.0))
            .padding([7.0, 12.0])
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
        if state.update_checking {
            button.into()
        } else {
            let action = if state.update_asset.is_some() {
                Message::DownloadUpdate
            } else {
                Message::OpenReleases
            };
            button.on_press(action).into()
        }
    } else {
        std_button(
            if state.update_checking {
                "检查中…"
            } else {
                "检查"
            },
            Message::CheckUpdate,
        )
    };

    flow_card(vec![
        flow_row(
            "Kite",
            "轻量 Windows 启动器".to_string(),
            text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(12.0)
                .color(TEXT_MUTED)
                .into(),
        ),
        flow_row("检查更新", hint, control),
        flow_row(
            "最新发布",
            "在 GitHub 下载官方安装包".to_string(),
            std_button("打开发布页", Message::OpenReleases),
        ),
        flow_row(
            "GitHub 仓库",
            "https://github.com/NGLSL/Kite".to_string(),
            std_button("访问仓库", Message::OpenRepository),
        ),
    ])
}

/// 提示条（css .settings-toast）：底部居中胶囊。
/// 导航/内容之间的竖向分隔线（css .settings-nav border-right）。
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

/// 1px 分隔线。
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
