//! 搜索结果区：网格、单列/双列结果卡片与滚动条。

use iced::widget::{
    column, container, image, mouse_area, row, scrollable, space::Space, text, MouseArea,
    Scrollable,
};
use iced::{alignment, border, color, Background, Border, Color, Element, Length, Padding, Theme};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::super::NavigationMode;
use super::panel::panel_area;
use super::text::{first_char, truncate_display_label, truncate_middle};
use super::{enabled_plugin_samples, provider_meta, scroll_id, Message, State};
use crate::model::AppItem;
pub(super) fn results_area<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    if let Some(panel) = &state.plugin_panel {
        return panel_area(state, panel, tokens);
    }

    if state.query.trim().is_empty() && state.provider_mode.is_none() {
        return grid_dashboard(state, tokens);
    }

    if state.results.is_empty() {
        if let Some(act) = &state.provider_mode {
            let (title, sub) = if let Some(err) = &state.plugin_flash {
                ("插件暂时不可用", err.as_str())
            } else if act.effective_query.trim().is_empty() {
                let meta = provider_meta(&act.provider_id);
                (meta.placeholder, "支持加减乘除、括号、乘方、函数等即时运算")
            } else {
                ("插件查询中…", "结果将显示在此处")
            };
            let empty = column![
                text(title)
                    .size(14.0)
                    .color(tokens.text_primary)
                    .font(name_font()),
                text(sub).size(12.0).color(tokens.text_muted),
            ]
            .spacing(6.0)
            .align_x(alignment::Alignment::Center);
            return container(empty)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Alignment::Center)
                .align_y(alignment::Alignment::Center)
                .padding(40.0)
                .into();
        }
        let samples = enabled_plugin_samples(state);
        let (title, sub) = if !state.query.trim().is_empty() {
            let sub = if samples.is_empty() {
                "试试其他关键词".to_string()
            } else {
                format!("也可试试：{}", samples.join(" · "))
            };
            ("没有找到相关结果", sub)
        } else if !state.index_ready {
            ("正在构建索引…", "很快就好，通常不到一秒".to_string())
        } else {
            ("", String::new())
        };
        if title.is_empty() {
            return Space::new().width(Length::Fill).height(Length::Fill).into();
        }
        let empty = column![
            text(title)
                .size(14.0)
                .color(tokens.text_primary)
                .font(name_font()),
            text(sub).size(12.0).color(tokens.text_muted),
        ]
        .spacing(6.0)
        .align_x(alignment::Alignment::Center);
        return container(empty)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Alignment::Center)
            .align_y(alignment::Alignment::Center)
            .padding(40.0)
            .into();
    }

    // 非空 Query：横向双列卡片流（2-Column Cards）
    two_column_results(state, tokens)
}

fn render_grid_row<'a>(
    state: &State,
    items: &'a [crate::model::SearchResult],
    start_idx: usize,
    count: usize,
    tokens: ThemeTokens,
    is_pinned: bool,
) -> Element<'a, Message> {
    let mut r = row![].spacing(6.0).width(Length::Fill);
    for col in 0..8 {
        let idx = start_idx + col;
        if col < count && idx < items.len() {
            r = r.push(grid_tile(state, &items[idx], idx, tokens, is_pinned));
        } else {
            r = r.push(Space::new().width(Length::FillPortion(1)));
        }
    }
    r.into()
}

/// zTools 风格横向高密度图标网格仪表盘（空 Query 状态）
fn grid_dashboard<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let mut main_col = column![].spacing(12.0);

    let recent_count = state.grid_recent_count.min(state.results.len());

    // 分区 1：“最近使用”（最多 2 排 × 8 列 = 16 项）
    if recent_count > 0 {
        let mut recent_col = column![].spacing(6.0);
        recent_col = recent_col.push(
            text("最近使用")
                .size(11.0)
                .font(name_font())
                .color(tokens.text_muted),
        );

        // Row 0: 0..8
        recent_col = recent_col.push(render_grid_row(
            state,
            &state.results,
            0,
            recent_count.min(8),
            tokens,
            false,
        ));

        // Row 1: 8..16
        if recent_count > 8 {
            recent_col = recent_col.push(render_grid_row(
                state,
                &state.results,
                8,
                recent_count - 8,
                tokens,
                false,
            ));
        }
        main_col = main_col.push(recent_col);
    }

    // 分区 2：“已固定”（一排，最多 8 项）
    if state.results.len() > state.grid_recent_count {
        let pinned_count = state.results.len() - state.grid_recent_count;
        let mut pinned_col = column![].spacing(6.0);
        pinned_col = pinned_col.push(
            text("已固定")
                .size(11.0)
                .font(name_font())
                .color(tokens.text_muted),
        );

        pinned_col = pinned_col.push(render_grid_row(
            state,
            &state.results,
            state.grid_recent_count,
            pinned_count.min(8),
            tokens,
            true,
        ));
        main_col = main_col.push(pinned_col);
    }

    container(main_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 14.0,
            right: 18.0,
            bottom: 12.0,
            left: 18.0,
        })
        .into()
}

/// 网格单项磁贴：36×36 高清透明图标 + 11px 小字标题 + 选中微光
fn grid_tile<'a>(
    state: &State,
    r: &'a crate::model::SearchResult,
    i: usize,
    tokens: ThemeTokens,
    _is_pinned: bool,
) -> Element<'a, Message> {
    let active = state.navigation_mode == NavigationMode::Results && i == state.selected;
    let icon = render_icon(&r.item, 36.0, tokens);

    let label = truncate_display_label(&r.item.display_name, 11);
    let title_text = text(label)
        .size(11.0)
        .font(name_font())
        .color(if active {
            tokens.text_primary
        } else {
            tokens.text_muted
        })
        .align_x(alignment::Alignment::Center);

    let content = column![icon, title_text]
        .spacing(6.0)
        .align_x(alignment::Alignment::Center);

    let active_bg = tokens.active_bg;
    let active_border = tokens.active_border;

    let tile = container(content)
        .width(Length::FillPortion(1))
        .height(68.0)
        .align_x(alignment::Alignment::Center)
        .align_y(alignment::Alignment::Center)
        .padding(4.0)
        .style(move |_t| container::Style {
            background: if active {
                Some(Background::Color(active_bg))
            } else {
                None
            },
            border: Border {
                color: if active {
                    active_border
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..container::Style::default()
        });

    let area: MouseArea<'_, Message> = mouse_area(tile)
        .on_enter(Message::HoverSelect(i))
        .on_press(Message::LaunchIndex(i))
        .on_right_press(Message::ContextMenu(i));

    area.into()
}

/// 横向双列卡片流（非空 Query 状态）：左右对称充分利用 640px 空间
fn two_column_results<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let mut list = column![].spacing(6.0);
    let n = state.results.len();
    let chunks = (n + 7) / 8;

    for c in 0..chunks {
        let base = c * 8;
        for r in 0..4 {
            let left_idx = base + r;
            let right_idx = base + r + 4;
            if left_idx >= n {
                break;
            }
            let left_elem = result_card(state, &state.results[left_idx], left_idx, tokens);
            let right_elem: Element<'_, Message> = if right_idx < n {
                result_card(state, &state.results[right_idx], right_idx, tokens)
            } else {
                Space::new().width(Length::FillPortion(1)).into()
            };
            let row_elem = row![left_elem, right_elem].spacing(8.0).width(Length::Fill);
            list = list.push(row_elem);
        }
    }

    let scroller: Scrollable<'_, Message> = scrollable(list)
        .id(scroll_id())
        .direction(scrollable::Direction::Vertical(results_scrollbar()))
        .style(results_scroll_style_tokens(tokens))
        .width(Length::Fill)
        .height(Length::Fill);

    container(scroller)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 9.0,
            right: 10.0,
            bottom: 9.0,
            left: 10.0,
        })
        .into()
}

/// 双列卡片项：左列 Alt 1~4，右列 Alt 5~8，带分类胶囊与实体键帽
fn result_card<'a>(
    state: &State,
    r: &'a crate::model::SearchResult,
    i: usize,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    let active = state.navigation_mode == NavigationMode::Results && i == state.selected;
    let icon = render_icon(&r.item, 36.0, tokens);

    let tag = match r.item.source.as_str() {
        "everything" | "direct-path" => Some("文件"),
        "plugin" | "plugin-command" => Some("插件"),
        "browser" | "websearch" => Some("网页"),
        _ => Some("应用"),
    };

    let title_str = truncate_display_label(&r.item.display_name, 14);
    let mut title_row = row![text(title_str)
        .size(13.5)
        .font(name_font())
        .color(tokens.text_primary)
        .wrapping(iced::widget::text::Wrapping::None),]
    .spacing(6.0)
    .align_y(alignment::Alignment::Center);

    if let Some(t) = tag {
        let tag_pill = container(
            text(t)
                .size(9.5)
                .color(tokens.accent)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .padding([1.0, 4.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(Color {
                a: 0.12,
                ..tokens.accent
            })),
            border: Border::default().rounded(3.0),
            ..container::Style::default()
        });
        title_row = title_row.push(tag_pill);
    }

    let subtitle = truncate_middle(&r.item.target, 28);
    let sub_text = text(subtitle)
        .size(11.0)
        .color(tokens.text_muted)
        .wrapping(iced::widget::text::Wrapping::None);

    let info_col = column![title_row, sub_text]
        .spacing(2.0)
        .width(Length::Fill);

    // 快捷键微立体实体键帽（Alt+1..4 左列，Alt+5..8 右列）
    let keycap_elem: Element<'_, Message> = if i < 8 {
        let keycap_str = format!("Alt+{}", i + 1);
        container(
            text(keycap_str)
                .size(9.5)
                .color(tokens.keycap_text)
                .font(name_font())
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .padding([2.0, 5.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.keycap_bg)),
            border: Border {
                color: tokens.keycap_border,
                width: 1.0,
                radius: border::radius(4.0),
            },
            ..container::Style::default()
        })
        .into()
    } else if state.pinned.contains(&r.item.id) {
        text("★").size(12.0).color(color!(0xF5_9E_0B)).into()
    } else {
        Space::new().into()
    };

    let content = row![icon, info_col, keycap_elem]
        .spacing(8.0)
        .align_y(alignment::Alignment::Center);

    let active_bg = tokens.active_bg;
    let bg_elevated = tokens.bg_elevated;
    let active_border = tokens.active_border;
    let border_subtle = tokens.border_subtle;

    let card = container(content)
        .width(Length::FillPortion(1))
        .height(58.0)
        .padding(Padding {
            top: 6.0,
            right: 12.0,
            bottom: 6.0,
            left: 10.0,
        })
        .style(move |_t| container::Style {
            background: Some(Background::Color(if active {
                active_bg
            } else {
                bg_elevated
            })),
            border: Border {
                color: if active { active_border } else { border_subtle },
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..container::Style::default()
        });

    let area: MouseArea<'_, Message> = mouse_area(card)
        .on_enter(Message::HoverSelect(i))
        .on_press(Message::LaunchIndex(i))
        .on_right_press(Message::ContextMenu(i));

    area.into()
}

/// 图标直出：移除灰色圆角方框，Linear 双线性插值滤波；官方插件与无图项采用精致微标
fn render_icon(item: &AppItem, size: f32, tokens: ThemeTokens) -> Element<'static, Message> {
    if let Some(path) = &item.icon {
        return container(
            image(image::Handle::from_path(path))
                .width(size)
                .height(size)
                .content_fit(iced::ContentFit::Contain)
                .filter_method(iced::widget::image::FilterMethod::Linear),
        )
        .width(size)
        .height(size)
        .align_x(alignment::Alignment::Center)
        .align_y(alignment::Alignment::Center)
        .into();
    }

    let id = item.id.to_lowercase();
    let name = item.display_name.to_lowercase();

    let (badge_text, bg_color, fg_color, font_size) =
        if id.contains("calc") || name.contains("计算") {
            (
                "=",
                Color {
                    a: 0.15,
                    ..tokens.accent
                },
                tokens.accent,
                18.0,
            )
        } else if id.contains("json") || name.contains("json") {
            (
                "{ }",
                Color {
                    a: 0.15,
                    ..color!(0xF5_9E_0B)
                },
                color!(0xF5_9E_0B),
                12.0,
            )
        } else if id.contains("ts") || name.contains("时间戳") {
            (
                "⏱",
                Color {
                    a: 0.15,
                    ..color!(0x10_B9_81)
                },
                color!(0x10_B9_81),
                14.0,
            )
        } else if id.contains("win") || name.contains("窗口") {
            (
                "⊞",
                Color {
                    a: 0.15,
                    ..color!(0x63_66_F1)
                },
                color!(0x63_66_F1),
                15.0,
            )
        } else if id.contains("file") || name.contains("文件") {
            (
                "📁",
                Color {
                    a: 0.15,
                    ..tokens.accent
                },
                tokens.accent,
                15.0,
            )
        } else if id.contains("settings") || name.contains("设置") {
            (
                "⚙",
                Color {
                    a: 0.15,
                    ..tokens.text_muted
                },
                tokens.text_muted,
                15.0,
            )
        } else {
            ("", tokens.bg_elevated, tokens.text_primary, 14.0)
        };

    let inner_elem: Element<'static, Message> = if badge_text.is_empty() {
        text(first_char(&item.display_name))
            .size(font_size)
            .font(name_font())
            .color(fg_color)
            .into()
    } else {
        text(badge_text)
            .size(font_size)
            .font(name_font())
            .color(fg_color)
            .into()
    };

    container(inner_elem)
        .width(size)
        .height(size)
        .align_x(alignment::Alignment::Center)
        .align_y(alignment::Alignment::Center)
        .style(move |_t| container::Style {
            background: Some(Background::Color(bg_color)),
            border: Border {
                color: Color {
                    a: 0.10,
                    ..fg_color
                },
                width: 1.0,
                radius: border::radius(8.0),
            },
            ..container::Style::default()
        })
        .into()
}

/// 结果列表滚动条：细圆角拇指、无轨道底
pub(crate) fn results_scrollbar() -> scrollable::Scrollbar {
    scrollable::Scrollbar::new()
        .width(8.0)
        .scroller_width(4.0)
        .margin(2.0)
}

/// 细滚动条样式（接入 ThemeTokens，自适应深色/浅色模式）
pub(crate) fn results_scroll_style_tokens(
    tokens: ThemeTokens,
) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style {
    move |_theme, status| {
        use iced::widget::scrollable::{AutoScroll, Rail, Scroller, Style};
        use iced::Vector;

        let hovering = matches!(
            status,
            scrollable::Status::Hovered {
                is_vertical_scrollbar_hovered: true,
                ..
            } | scrollable::Status::Dragged {
                is_vertical_scrollbar_dragged: true,
                ..
            }
        );
        let thumb = if hovering {
            Color {
                a: 0.55,
                ..tokens.text_muted
            }
        } else {
            Color {
                a: 0.25,
                ..tokens.text_muted
            }
        };
        let rail = Rail {
            background: None,
            border: Border::default(),
            scroller: Scroller {
                background: Background::Color(thumb),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: border::radius(999.0),
                },
            },
        };
        Style {
            container: container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
            auto_scroll: AutoScroll {
                background: Background::Color(Color {
                    a: 0.92,
                    ..tokens.bg_elevated
                }),
                border: Border {
                    color: tokens.border_window,
                    width: 1.0,
                    radius: border::radius(999.0),
                },
                shadow: iced::Shadow {
                    color: Color {
                        a: 0.18,
                        ..tokens.text_primary
                    },
                    offset: Vector::ZERO,
                    blur_radius: 6.0,
                },
                icon: tokens.text_muted,
            },
        }
    }
}
