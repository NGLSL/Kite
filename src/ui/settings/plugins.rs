//! 插件管理：设置内列表 + 说明详情；工具（JSON）走独立面板。

use iced::font::{Font, Weight};
use iced::widget::{button, column, container, row, space::Space, text, text_input};
use iced::{alignment, border, color, Background, Border, Color, Element, Length};

use super::super::font::{name_font, ui_font};
use super::super::search_view::{BG_PANEL, BORDER, MARK, TEXT, TEXT_MUTED};
use super::super::{Message, State};
use super::widgets::{divider, elevated_card, ghost_button, primary_button, status_pill, toggle};

pub(super) fn plugins_card(state: &State) -> Element<'_, Message> {
    if let Some(id) = state.plugin_docs_open.clone() {
        return plugin_detail_page(state, &id);
    }
    plugins_list_page(state)
}

fn plugins_list_page(state: &State) -> Element<'_, Message> {
    let reg = state
        .plugin_registry
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let host = state
        .plugin_host
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // 产品区：先能力，后维护。导入/维护沉底，不再压住插件列表。
    let mut sections: Vec<Element<'_, Message>> = Vec::new();

    // 非内联工具：二次确认后才开独立窗；内联插件无此步。
    let json_tool_action: Element<'_, Message> = if state
        .pending_tool_confirm
        .as_ref()
        .is_some_and(|p| p.from_settings)
    {
        row![
            text("确认打开独立 JSON 工具窗？")
                .size(12.0)
                .color(TEXT)
                .width(Length::Fill),
            primary_button("确认打开".into(), Message::PluginConfirmJsonTool),
            ghost_button("取消".into(), Message::PluginCancelJsonToolConfirm, false),
        ]
        .spacing(8.0)
        .align_y(alignment::Alignment::Center)
        .into()
    } else {
        primary_button("打开面板".into(), Message::PluginOpenJsonTool).into()
    };

    sections.push(elevated_card(
        column![
            row![
                tool_glyph("{}"),
                column![
                    text("JSON 工具")
                        .size(15.0)
                        .color(TEXT)
                        .font(name_font()),
                    text("独立面板：左原始 · 右格式化/压缩 · 打开前需确认")
                        .size(12.0)
                        .color(TEXT_MUTED),
                ]
                .spacing(2.0)
                .width(Length::Fill),
            ]
            .spacing(12.0)
            .align_y(alignment::Alignment::Center),
            json_tool_action,
        ]
        .spacing(10.0)
        .into(),
    ));

    if reg.is_empty() {
        sections.push(elevated_card(
            column![
                text("已安装插件").size(13.0).color(TEXT).font(name_font()),
                text("暂无插件。可在下方「重装官方」装入计算器、窗口切换、开发者工具；或在搜索框直接试用。")
                    .size(12.0)
                    .color(TEXT_MUTED),
                row![
                    ghost_button("回到搜索".into(), Message::PluginTryExample(String::new()), false),
                    ghost_button("重装官方".into(), Message::PluginInstallOfficial, false),
                ]
                .spacing(8.0)
                .align_y(alignment::Alignment::Center),
            ]
            .spacing(8.0)
            .into(),
        ));
    } else {
        sections.push(
            column![
                row![
                    text("已安装插件")
                        .size(13.0)
                        .color(TEXT)
                        .font(name_font()),
                    text(format!("{}", reg.iter().count()))
                        .size(12.0)
                        .color(TEXT_MUTED),
                    Space::new().width(Length::Fill),
                    ghost_button(
                        "回到搜索".into(),
                        Message::PluginTryExample(String::new()),
                        false,
                    ),
                ]
                .spacing(8.0)
                .align_y(alignment::Alignment::Center)
                .width(Length::Fill),
            ]
            .spacing(8.0)
            .width(Length::Fill)
            .into(),
        );
        for plugin in reg.iter() {
            sections.push(plugin_card(plugin, &host));
        }
    }

    let import_input = text_input(
        r"插件文件夹路径（含 plugin.json）",
        &state.plugin_import_path,
    )
    .on_input(Message::PluginImportPathChanged)
    .on_submit(Message::PluginImportFromPath)
    .size(12.5)
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

    sections.push(elevated_card(
        column![
            text("导入与维护")
                .size(12.5)
                .color(TEXT_MUTED)
                .font(name_font()),
            row![
                import_input,
                ghost_button("导入".into(), Message::PluginImportFromPath, false),
            ]
            .spacing(8.0)
            .align_y(alignment::Alignment::Center)
            .width(Length::Fill),
            row![
                ghost_button("重装官方".into(), Message::PluginInstallOfficial, false),
                ghost_button("打开目录".into(), Message::PluginOpenPluginsDir, false),
                ghost_button("重新扫描".into(), Message::PluginRescanPlugins, false),
            ]
            .spacing(8.0)
            .align_y(alignment::Alignment::Center),
        ]
        .spacing(8.0)
        .width(Length::Fill)
        .into(),
    ));

    column(sections).spacing(10.0).width(Length::Fill).into()
}

/// 列表页用户态文案：正常启用显示「可用」，异常/停用才展开技术状态。
fn list_status(phase: crate::plugin::RuntimePhase, enabled: bool, last_error: Option<&str>, host_fault: Option<&str>) -> (String, bool) {
    if !enabled {
        return ("已停用".into(), false);
    }
    let label = crate::plugin::runtime_phase_label(phase, last_error, host_fault);
    let bad = label.contains("崩溃")
        || label.contains("故障")
        || label.contains("不兼容")
        || last_error.is_some_and(|e| e.contains("crash loop"));
    if bad {
        (label, true)
    } else {
        ("可用".into(), false)
    }
}

fn tool_glyph(label: &'static str) -> Element<'static, Message> {
    container(
        text(label)
            .size(13.0)
            .font(Font {
                weight: Weight::Semibold,
                ..ui_font()
            })
            .color(MARK),
    )
    .width(36.0)
    .height(36.0)
    .align_x(alignment::Alignment::Center)
    .align_y(alignment::Alignment::Center)
    .style(|_t| container::Style {
        background: Some(Background::Color(color!(0xEF_F6_FF))),
        border: Border::default().rounded(9.0),
        ..container::Style::default()
    })
    .into()
}

fn plugin_card(
    plugin: &crate::plugin::registry::RegisteredPlugin,
    host: &crate::plugin::PluginHost,
) -> Element<'static, Message> {
    let id = plugin.manifest.plugin.id.clone();
    let name = crate::plugin::display_plugin_name(&plugin.manifest);
    let version = plugin.manifest.plugin.version.clone();
    let blurb = crate::plugin::display_plugin_blurb(&plugin.manifest);
    let examples = crate::plugin::try_examples(&plugin.manifest);
    let host_fault = match host.state(&id) {
        crate::plugin::PluginRuntimeState::Faulted { reason } => Some(reason.clone()),
        _ => None,
    };
    let (label, status_bad) = list_status(
        plugin.phase,
        plugin.enabled,
        plugin.last_error.as_deref(),
        host_fault.as_deref(),
    );
    let enabled = plugin.enabled;
    let detail = plugin
        .last_error
        .clone()
        .or_else(|| host_fault.map(|s| s.to_string()))
        .unwrap_or_default();

    let glyph = first_char(&name);
    let icon = container(text(glyph).size(14.0).font(name_font()).color(TEXT_MUTED))
        .width(36.0)
        .height(36.0)
        .align_x(alignment::Alignment::Center)
        .align_y(alignment::Alignment::Center)
        .style(|_t| container::Style {
            background: Some(Background::Color(color!(0xF1_F2_F4))),
            border: Border::default().rounded(9.0),
            ..container::Style::default()
        });

    let title = row![
        text(name)
            .size(14.5)
            .color(TEXT)
            .font(name_font()),
        text(format!("v{version}")).size(11.0).color(TEXT_MUTED),
        status_pill(&label, status_bad, enabled),
        Space::new().width(Length::Fill),
        toggle(enabled, Message::PluginSetEnabled(id.clone(), !enabled)),
    ]
    .spacing(8.0)
    .align_y(alignment::Alignment::Center)
    .width(Length::Fill);

    let mut body = column![
        title,
        text(blurb)
            .size(12.0)
            .color(TEXT_MUTED)
            .width(Length::Fill),
    ]
    .spacing(5.0)
    .width(Length::Fill);

    if !examples.is_empty() {
        let mut chips = row![].spacing(6.0).align_y(alignment::Alignment::Center);
        for ex in examples.iter().take(2) {
            chips = chips.push(example_chip(ex.clone()));
        }
        body = body.push(chips);
    }

    // 列表只保留高频动作：说明 / 重载。目录·日志·卸载进详情页。
    body = body.push(
        row![
            Space::new().width(Length::Fill),
            ghost_button("说明".into(), Message::PluginToggleDocs(id.clone()), false),
            ghost_button("重载".into(), Message::PluginReload(id), false),
        ]
        .spacing(8.0)
        .align_y(alignment::Alignment::Center)
        .width(Length::Fill),
    );

    if !detail.is_empty() {
        body = body.push(text(detail).size(11.0).color(color!(0xDC_26_26)));
    }

    elevated_card(
        row![icon, column![body].width(Length::Fill)]
            .spacing(12.0)
            .align_y(alignment::Alignment::Start)
            .width(Length::Fill)
            .into(),
    )
}

/// 说明详情：整页文档，不是行内弹层。
fn plugin_detail_page(state: &State, plugin_id: &str) -> Element<'static, Message> {
    let reg = state
        .plugin_registry
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let host = state
        .plugin_host
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let Some(plugin) = reg.get(plugin_id) else {
        return column![
            ghost_button(
                "← 返回列表".into(),
                Message::PluginToggleDocs(plugin_id.to_string()),
                false
            ),
            text("插件不存在或已卸载").size(13.0).color(TEXT_MUTED),
        ]
        .spacing(12.0)
        .into();
    };

    let id = plugin.manifest.plugin.id.clone();
    let name = crate::plugin::display_plugin_name(&plugin.manifest);
    let version = plugin.manifest.plugin.version.clone();
    let blurb = crate::plugin::display_plugin_blurb(&plugin.manifest);
    let usage = crate::plugin::display_plugin_usage(&plugin.manifest);
    let examples = crate::plugin::try_examples(&plugin.manifest);
    let host_fault = match host.state(&id) {
        crate::plugin::PluginRuntimeState::Faulted { reason } => Some(reason.clone()),
        _ => None,
    };
    let enabled = plugin.enabled;
    let detail = plugin
        .last_error
        .clone()
        .or_else(|| host_fault.clone())
        .unwrap_or_default();
    let (label, status_bad) = list_status(
        plugin.phase,
        enabled,
        plugin.last_error.as_deref(),
        host_fault.as_deref(),
    );
    // 详情页可展示完整运行阶段（待命/运行中…），异常时与列表一致标红。
    let tech_label = if status_bad || !enabled {
        label.clone()
    } else {
        crate::plugin::runtime_phase_label(
            plugin.phase,
            plugin.last_error.as_deref(),
            host_fault.as_deref(),
        )
    };

    let header = row![
        ghost_button(
            "← 返回列表".into(),
            Message::PluginToggleDocs(id.clone()),
            false
        ),
        Space::new().width(Length::Fill),
        text("插件说明").size(12.0).color(TEXT_MUTED),
    ]
    .spacing(8.0)
    .align_y(alignment::Alignment::Center);

    let mut title_extra = row![
        text(name).size(20.0).color(TEXT).font(name_font()),
        text(format!("v{version}")).size(12.0).color(TEXT_MUTED),
        status_pill(&tech_label, status_bad, enabled),
    ]
    .spacing(10.0)
    .align_y(alignment::Alignment::Center);

        if id == "com.kite.devtools" {
            title_extra = title_extra.push(Space::new().width(Length::Fill));
            if state
                .pending_tool_confirm
                .as_ref()
                .is_some_and(|p| p.from_settings)
            {
                title_extra = title_extra.push(primary_button(
                    "确认打开 JSON 工具".into(),
                    Message::PluginConfirmJsonTool,
                ));
                title_extra = title_extra.push(ghost_button(
                    "取消".into(),
                    Message::PluginCancelJsonToolConfirm,
                    false,
                ));
            } else {
                title_extra = title_extra.push(primary_button(
                    "打开 JSON 面板".into(),
                    Message::PluginOpenJsonTool,
                ));
            }
        }

    let mut usage_col = column![
        section_label("用法"),
        text(usage).size(13.0).color(TEXT).width(Length::Fill),
    ]
    .spacing(8.0)
    .width(Length::Fill);

    if !examples.is_empty() {
        let mut chips = row![].spacing(8.0).align_y(alignment::Alignment::Center);
        for ex in examples.iter().take(6) {
            chips = chips.push(example_chip(ex.clone()));
        }
        usage_col = usage_col.push(section_label("示例（点击试用）"));
        usage_col = usage_col.push(chips);
    }

    let mut manage_col = column![].spacing(8.0).width(Length::Fill);
    if !detail.is_empty() {
        manage_col = manage_col.push(text(detail).size(11.0).color(color!(0xDC_26_26)));
    }
    manage_col = manage_col.push(
        row![
            toggle(enabled, Message::PluginSetEnabled(id.clone(), !enabled)),
            ghost_button("重载".into(), Message::PluginReload(id.clone()), false),
            ghost_button("目录".into(), Message::PluginOpenDir(id.clone()), false),
            ghost_button("日志".into(), Message::PluginOpenLog(id.clone()), false),
            ghost_button("卸载".into(), Message::PluginUninstall(id.clone()), true),
        ]
        .spacing(8.0)
        .align_y(alignment::Alignment::Center),
    );

    let card = elevated_card(
        column![
            title_extra,
            text(blurb).size(13.0).color(TEXT).width(Length::Fill),
            divider(),
            usage_col,
            divider(),
            section_label("管理"),
            manage_col,
            divider(),
            column![
                section_label("标识"),
                text(id.clone()).size(12.0).color(TEXT_MUTED),
            ]
            .spacing(6.0),
        ]
        .spacing(12.0)
        .width(Length::Fill)
        .into(),
    );

    column![header, card]
        .spacing(12.0)
        .width(Length::Fill)
        .into()
}

fn section_label(title: &str) -> Element<'static, Message> {
    text(title.to_string())
        .size(12.0)
        .color(MARK)
        .font(name_font())
        .into()
}

fn example_chip(example: String) -> Element<'static, Message> {
    button(text(example.clone()).size(11.5).color(MARK))
        .padding([4.0, 8.0])
        .on_press(Message::PluginTryExample(example))
        .style(|_t, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                color!(0xDB_EAFE)
            } else {
                color!(0xEF_F6_FF)
            })),
            text_color: MARK,
            border: Border {
                color: color!(0xBF_DB_FE),
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..button::Style::default()
        })
        .into()
}

fn first_char(s: &str) -> String {
    s.chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into())
}
