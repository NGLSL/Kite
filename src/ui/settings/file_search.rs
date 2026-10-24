use iced::widget::{column, row, space::Space, text, text_input};
use iced::{alignment, Element, Length};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::{Message, Section, State};
use super::widgets::{divider, elevated_card, ghost_button, primary_button, status_pill, toggle};

/// 插件页上的轻量入口：只展示能力状态和管理入口。
pub(super) fn plugin_card<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let snapshot = state.file_search_controller.snapshot();
    let enabled = state.file_search_enabled;
    let status = status_label(&snapshot, enabled);
    let status_bad = matches!(
        snapshot.phase,
        crate::file_search::Phase::Error
            | crate::file_search::Phase::NeedsAuthorization
            | crate::file_search::Phase::NeedsRebuild
    );

    let body = column![row![
        text("文件搜索")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font()),
        text("内置").size(11.0).color(tokens.text_muted),
        status_pill(&status, status_bad, enabled, tokens),
        Space::new().width(Length::Fill),
        ghost_button(
            "管理".into(),
            Message::SettingsSection(Section::FileSearch),
            false,
            tokens,
        ),
        toggle(enabled, Message::FileSearchSetEnabled(!enabled), tokens),
    ]
    .spacing(6.0)
    .align_y(alignment::Alignment::Center)
    .width(Length::Fill),]
    .spacing(6.0)
    .width(Length::Fill);

    elevated_card(body.into(), tokens)
}

/// 设置页“文件搜索”分区：启停、授权、进度、重建和排除目录集中管理。
pub(super) fn settings_card<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let snapshot = state.file_search_controller.snapshot();
    let enabled = state.file_search_enabled;
    let status = status_label(&snapshot, enabled);
    let status_bad = matches!(
        snapshot.phase,
        crate::file_search::Phase::Error
            | crate::file_search::Phase::NeedsAuthorization
            | crate::file_search::Phase::NeedsRebuild
    );

    let mut body = column![row![
        text("文件搜索")
            .size(13.0)
            .color(tokens.text_primary)
            .font(name_font()),
        text("内置").size(11.0).color(tokens.text_muted),
        status_pill(&status, status_bad, enabled, tokens),
        Space::new().width(Length::Fill),
        toggle(enabled, Message::FileSearchSetEnabled(!enabled), tokens),
    ]
    .spacing(6.0)
    .align_y(alignment::Alignment::Center),]
    .spacing(6.0)
    .width(Length::Fill);

    if state.file_search_confirm_open {
        body = body.push(
            elevated_card(
                column![
                    text("启用文件搜索")
                        .size(12.0)
                        .color(tokens.text_primary)
                        .font(name_font()),
                    text("首次使用需管理员授权安装索引服务，仅读取文件名和目录变化，不读取文件内容。已有索引会自动复用；关闭文件搜索或退出 Kite 后停止读取。")
                        .size(12.0)
                        .color(tokens.text_muted)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                    row![
                        primary_button(
                            "启用文件搜索".into(),
                            Message::FileSearchEnableConfirm,
                            tokens,
                        ),
                        ghost_button(
                            "暂不启用".into(),
                            Message::FileSearchEnableCancel,
                            false,
                            tokens,
                        ),
                    ]
                    .spacing(6.0)
                    .align_y(alignment::Alignment::Center),
                ]
                .spacing(6.0)
                .into(),
                tokens,
            ),
        );
    } else if enabled {
        let detail = if snapshot.entries > 0 && !snapshot.detail.trim().is_empty() {
            format!("{} · {} 条", snapshot.detail, snapshot.entries)
        } else if snapshot.entries > 0 {
            format!("已索引 {} 条", snapshot.entries)
        } else if snapshot.detail.trim().is_empty() {
            "等待索引进度…".to_string()
        } else {
            snapshot.detail.clone()
        };
        body = body.push(
            text(detail)
                .size(11.5)
                .color(tokens.text_muted)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
        let building = matches!(
            snapshot.phase,
            crate::file_search::Phase::Starting
                | crate::file_search::Phase::Indexing
                | crate::file_search::Phase::CatchingUp
                | crate::file_search::Phase::Recovering
        );
        let actions = if building {
            row![
                Space::new().width(Length::Fill),
                ghost_button(
                    "关闭文件搜索".into(),
                    Message::FileSearchSetEnabled(false),
                    false,
                    tokens,
                ),
            ]
            .spacing(6.0)
        } else {
            let phase_actions = match snapshot.phase {
                crate::file_search::Phase::RetryWaiting => row![ghost_button(
                    "立即重试".into(),
                    Message::FileSearchRetry,
                    false,
                    tokens,
                )]
                .spacing(6.0),
                crate::file_search::Phase::Error => row![
                    ghost_button("重试连接".into(), Message::FileSearchRetry, false, tokens,),
                    ghost_button("重建索引".into(), Message::FileSearchRebuild, false, tokens),
                ]
                .spacing(6.0),
                crate::file_search::Phase::NeedsAuthorization => row![ghost_button(
                    "授权启用".into(),
                    Message::FileSearchSetEnabled(true),
                    false,
                    tokens,
                )]
                .spacing(6.0),
                crate::file_search::Phase::NeedsRebuild => row![ghost_button(
                    "重试恢复".into(),
                    Message::FileSearchRebuild,
                    false,
                    tokens,
                )]
                .spacing(6.0),
                crate::file_search::Phase::Ready => row![ghost_button(
                    "重建索引".into(),
                    Message::FileSearchRebuild,
                    false,
                    tokens,
                )]
                .spacing(6.0),
                crate::file_search::Phase::Stopped => row![ghost_button(
                    "重试连接".into(),
                    Message::FileSearchRetry,
                    false,
                    tokens,
                )]
                .spacing(6.0),
                crate::file_search::Phase::Starting
                | crate::file_search::Phase::Indexing
                | crate::file_search::Phase::CatchingUp
                | crate::file_search::Phase::Recovering => row![].spacing(6.0),
            };
            row![
                phase_actions,
                Space::new().width(Length::Fill),
                ghost_button(
                    "关闭文件搜索".into(),
                    Message::FileSearchSetEnabled(false),
                    false,
                    tokens,
                ),
            ]
            .spacing(6.0)
        };
        body = body.push(actions);
    }

    if enabled {
        body = body.push(divider(tokens));
        body = body.push(
            text("排除文件夹")
                .size(12.0)
                .color(tokens.text_primary)
                .font(name_font()),
        );
        body = body.push(
            text("排除目录及其子树会立即从搜索结果隐藏，不会为此自动重建索引。")
                .size(11.5)
                .color(tokens.text_muted)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
        let input = text_input(r"例如 C:\Work\Cache", &state.file_search_exclusion_input)
            .on_input(Message::FileSearchExcludeInputChanged)
            .on_submit(Message::FileSearchExcludeAdd)
            .size(12.0)
            .padding([6.0, 9.0])
            .width(Length::Fill)
            .style(super::widgets::text_input_style(tokens));
        body = body.push(
            row![
                input,
                ghost_button("添加".into(), Message::FileSearchExcludeAdd, false, tokens)
            ]
            .spacing(6.0)
            .align_y(alignment::Alignment::Center),
        );
        if state.file_search_exclusions.is_empty() {
            body = body.push(text("暂无排除文件夹").size(11.5).color(tokens.text_muted));
        } else {
            for (index, path) in state.file_search_exclusions.iter().enumerate() {
                body = body.push(
                    row![
                        text(path.clone())
                            .size(11.5)
                            .color(tokens.text_primary)
                            .width(Length::Fill)
                            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                        ghost_button(
                            "移除".into(),
                            Message::FileSearchExcludeRemove(index),
                            false,
                            tokens,
                        ),
                    ]
                    .spacing(6.0)
                    .align_y(alignment::Alignment::Center),
                );
            }
        }
    }

    elevated_card(body.into(), tokens)
}

pub(super) fn status_label(snapshot: &crate::file_search::Snapshot, enabled: bool) -> String {
    if !enabled {
        return "已关闭".into();
    }
    match snapshot.phase {
        crate::file_search::Phase::Stopped => "已停止".into(),
        crate::file_search::Phase::Starting => "正在启动".into(),
        crate::file_search::Phase::Indexing => "正在建立索引".into(),
        crate::file_search::Phase::CatchingUp => "正在追平变化".into(),
        crate::file_search::Phase::Recovering => "正在恢复".into(),
        crate::file_search::Phase::RetryWaiting => "等待重试".into(),
        crate::file_search::Phase::Ready => "已就绪".into(),
        crate::file_search::Phase::NeedsAuthorization => "需要授权".into(),
        crate::file_search::Phase::NeedsRebuild => "恢复未完成".into(),
        crate::file_search::Phase::Error => "发生错误".into(),
    }
}
