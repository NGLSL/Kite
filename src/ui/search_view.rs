//! 原型 UI：按 Kite 前端（src/features/search/search.css + styles/global.css）1:1 复刻。
//! 设计 token 与前端 CSS 变量一致（浅色主题）。

use iced::widget::text_input;
use iced::widget::{
    button, canvas, column, container, image, mouse_area, row, scrollable, space::Space, stack,
    text, Button, Id as WidgetId, MouseArea, Scrollable,
};
use iced::{alignment, border, color, Background, Border, Color, Element, Length, Padding, Theme};

use super::font::name_font;
use super::{MenuAction, Message, State};
use crate::plugin::{PanelBlock, PanelData};

// ── 设计 token（global.css）──
pub(crate) const BG_PANEL: Color = color!(0xFF_FF_FF);
pub(crate) const BG_ELEVATED: Color = color!(0xF7_F8_FA);
pub(crate) const BORDER: Color = color!(0xD1_D5_DB);
pub(crate) const WINDOW_BORDER: Color = color!(0x9C_A3_AF);
pub(crate) const TEXT: Color = color!(0x11_18_27);
pub(crate) const TEXT_MUTED: Color = color!(0x6B_72_80);
pub(crate) const ACCENT_BG: Color = color!(0xEF_F6_FF);
pub(crate) const MARK: Color = color!(0x25_63_EB);
/// item-icon 背景：color-mix(text 5%, white)
pub(crate) const ICON_BG: Color = color!(0xF1_F2_F4);

/// 行步进 = 行高 52 + 间距 2（css .item min-height 52 / .results gap 2）。
pub const ROW_STEP: f32 = 54.0;
pub const SCROLL_ID: &str = "poc-results";
const INPUT_ID: &str = "poc-input";

pub fn input_id() -> WidgetId {
    WidgetId::new(INPUT_ID)
}

pub fn scroll_id() -> WidgetId {
    WidgetId::new(SCROLL_ID)
}

pub fn view(state: &State) -> Element<'_, Message> {
    // 窗口内衬 1px 外框（css .shell inset 0 0 0 1px window-border）
    let panel = column![
        search_row(state),
        divider(),
        results_area(state),
        divider(),
        footer_bar(state),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let root = container(panel)
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

    // 右键菜单以 overlay 叠加；无菜单时 overlay 为空，保持 Scrollable 在树中的位置不变。
    let root: Element<'_, Message> = root.into();
    let overlay: Element<'_, Message> = match &state.menu {
        Some((item, x, y)) => menu_overlay(state, item, *x, *y),
        None => Space::new().width(Length::Fill).height(Length::Fill).into(),
    };
    stack![root, overlay].into()
}

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

/// 搜索条（css .search-row）：padding 16/18、bg-elevated、底部 1px 分隔。
/// 放大镜与顶部 6px 边缘是拖拽区（css data-tauri-drag-region 的对应实现）。
fn search_row(state: &State) -> Element<'_, Message> {
    let magnifier =
        mouse_area(canvas(Magnifier).width(20.0).height(20.0)).on_press(Message::DragWindow);
    let placeholder = search_placeholder(state);
    let input = text_input(&placeholder, &state.query)
        .id(input_id())
        .on_input(Message::QueryChanged)
        .size(17.0)
        .style(|_t, _s| text_input::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            icon: TEXT_MUTED,
            placeholder: TEXT_MUTED,
            value: TEXT,
            selection: Color { a: 0.25, ..MARK },
        });

    let mut row_content = row![
        magnifier,
        input.width(Length::Fill),
        // css .files-toggle：Everything 文件搜索开关（胶囊）
        button(
            text("文件")
                .size(12.0)
                .color(if state.files_mode { MARK } else { TEXT_MUTED })
        )
        .padding([5.0, 10.0])
        .on_press(Message::ToggleFiles)
        .style(move |_t, _s| button::Style {
            background: if state.files_mode {
                Some(Background::Color(Color { a: 0.12, ..MARK }))
            } else {
                None
            },
            text_color: if state.files_mode { MARK } else { TEXT_MUTED },
            border: Border {
                color: if state.files_mode {
                    Color { a: 0.45, ..MARK }
                } else {
                    BORDER
                },
                width: 1.0,
                radius: border::radius(999.0),
            },
            ..button::Style::default()
        }),
    ]
    .spacing(12)
    .align_y(alignment::Alignment::Center);
    if !state.query.is_empty() {
        let clear: Button<Message> = button(
            text("×")
                .size(18.0)
                .color(TEXT_MUTED)
                .align_x(alignment::Alignment::Center),
        )
        .padding([4.0, 7.0])
        .on_press(Message::ClearQuery)
        .style(|_t, status| button::Style {
            background: if status == button::Status::Hovered {
                Some(Background::Color(Color { a: 0.08, ..TEXT }))
            } else {
                None
            },
            text_color: if status == button::Status::Hovered {
                TEXT
            } else {
                TEXT_MUTED
            },
            border: Border::default().rounded(6.0),
            ..button::Style::default()
        });
        row_content = row_content.push(clear);
    }

    let styled = container(row_content)
        .width(Length::Fill)
        .padding([16.0, 18.0])
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            ..container::Style::default()
        });

    // 顶部 6px 拖拽条（叠在搜索条上缘）
    let drag_strip =
        mouse_area(Space::new().width(Length::Fill).height(6.0)).on_press(Message::DragWindow);

    stack![styled, drag_strip].into()
}

/// 结果区（css .results）：padding 8、行间 2；空 query 列默认列表，
/// 有 query 无结果时显示空态（css .empty-state）。Provider Panel 时原生渲染声明式块。
fn results_area(state: &State) -> Element<'_, Message> {
    if let Some(panel) = &state.plugin_panel {
        return panel_area(state, panel);
    }

    if state.results.is_empty() {
        if state.provider_mode.is_some() {
            let (title, sub) = if let Some(err) = &state.plugin_flash {
                ("插件暂时不可用", err.as_str())
            } else {
                ("插件查询中…", "结果将显示在此处")
            };
            let empty = column![
                text(title).size(14.0).color(TEXT).font(name_font()),
                text(sub).size(12.0).color(TEXT_MUTED),
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
            text(title).size(14.0).color(TEXT).font(name_font()),
            text(sub).size(12.0).color(TEXT_MUTED),
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

    let mut list = column![].spacing(2.0);
    for (i, r) in state.results.iter().enumerate() {
        let pinned = state.pinned.contains(&r.item.id);
        list = list.push(item_row(state, r, i, pinned));
    }
    let scroller: Scrollable<'_, Message> = scrollable(list)
        .id(scroll_id())
        .direction(scrollable::Direction::Vertical(results_scrollbar()))
        .style(results_scroll_style)
        .width(Length::Fill)
        .height(Length::Fill);
    // 右侧 gutter：细滚动条叠在内容上时仍给 Alt+N 留净空。
    container(scroller)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 8.0,
            right: 14.0,
            bottom: 8.0,
            left: 8.0,
        })
        .into()
}

/// Provider Mode 声明式 Panel：text/value/key_value/notice/divider，Iced 原生渲染。
fn panel_area<'a>(state: &State, panel: &PanelData) -> Element<'a, Message> {
    let mut col = column![].spacing(8.0);

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
                    .color(TEXT_MUTED),
                text("Esc 退出 Provider").size(11.0).color(TEXT_MUTED),
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
                    "secondary" | "muted" => TEXT_MUTED,
                    "error" => color!(0xDC_26_26),
                    _ => TEXT,
                };
                col = col.push(text(t.clone()).size(14.0).color(c).width(Length::Fill));
            }
            PanelBlock::Value {
                label,
                value,
                selectable: _,
            } => {
                if let Some(l) = label {
                    col = col.push(text(l.clone()).size(12.0).color(TEXT_MUTED));
                }
                col = col.push(
                    text(value.clone())
                        .size(28.0)
                        .font(name_font())
                        .color(TEXT)
                        .width(Length::Fill),
                );
            }
            PanelBlock::KeyValue { items } => {
                let mut kv = column![].spacing(4.0);
                for it in items {
                    kv = kv.push(
                        row![
                            text(it.key.clone())
                                .size(12.0)
                                .color(TEXT_MUTED)
                                .width(Length::Fill),
                            text(it.value.clone()).size(12.0).color(TEXT),
                        ]
                        .spacing(12.0),
                    );
                }
                col = col.push(kv.width(Length::Fill));
            }
            PanelBlock::Notice { level, text: t } => {
                let (bg, fg) = match level.as_str() {
                    "error" => (color!(0xFE_F2_F2), color!(0xDC_26_26)),
                    "warning" => (color!(0xFF_F7_ED), color!(0xC2_41_0C)),
                    _ => (ACCENT_BG, MARK),
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
            PanelBlock::Divider => col = col.push(divider()),
        }
    }

    if let Some(action) = panel.actions.iter().find(|a| a.default) {
        col = col.push(
            text(format!("Enter · {}", action.label))
                .size(12.0)
                .color(TEXT_MUTED),
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

/// 单行结果（css .item）：52px 高、radius 10、icon 34、名称 15/500、
/// 右侧 Alt+N 提示；active 时 accent 背景 + mark 描边 + 左侧 3px mark 竖条。
fn item_row<'a>(
    state: &State,
    r: &'a crate::model::SearchResult,
    i: usize,
    pinned: bool,
) -> Element<'a, Message> {
    let active = i == state.selected;
    let icon: Element<'_, Message> = match &r.item.icon {
        Some(path) => image(image::Handle::from_path(path))
            .width(34.0)
            .height(34.0)
            // tiny-skia 默认最近邻插值 → 马赛克；64px 源图缩到 34 需要双线性
            .filter_method(iced::widget::image::FilterMethod::Linear)
            .into(),
        None => text(first_char(&r.item.display_name))
            .size(14.0)
            .font(name_font())
            .color(TEXT_MUTED)
            .into(),
    };
    let icon_box = container(icon)
        .width(34.0)
        .height(34.0)
        .align_x(alignment::Alignment::Center)
        .align_y(alignment::Alignment::Center)
        .style(|_t| container::Style {
            background: Some(Background::Color(ICON_BG)),
            border: Border::default().rounded(9.0),
            ..container::Style::default()
        });

    // 结果只展示用户可识别的名称；AUMID、exe 名和路径仍保存在 item 中供启动和右键操作。
    // 选中/hover 时第二行弱显示 target 路径；行高保持 52px，不因路径动态增高。
    let name_line = text(r.item.display_name.clone())
        .size(15.0)
        .font(name_font())
        .color(TEXT)
        .width(Length::Fill);
    let body: Element<'_, Message> = if active {
        let path = truncate_middle(&r.item.target, 56);
        column![
            name_line,
            text(path).size(11.0).color(TEXT_MUTED).width(Length::Fill)
        ]
        .spacing(1.0)
        .into()
    } else {
        name_line.into()
    };

    let hint_text = if i < 9 {
        if pinned {
            format!("已固定 Alt+{}", i + 1)
        } else {
            format!("Alt+{}", i + 1)
        }
    } else if pinned {
        "已固定".to_string()
    } else {
        String::new()
    };
    let hint_color = if active { MARK } else { TEXT_MUTED };
    let hint = text(hint_text).size(11.0).color(hint_color);

    let content = row![icon_box, body, hint]
        .spacing(12.0)
        .align_y(alignment::Alignment::Center);

    // 左侧 3px 高亮条（css .item.active::before，上下缩进 10px）；非选中为空占位
    let bar: Element<'_, Message> = if active {
        container(
            container(Space::new().width(3.0).height(32.0)).style(|_t: &Theme| container::Style {
                background: Some(Background::Color(MARK)),
                border: Border::default().rounded(2.0),
                ..container::Style::default()
            }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Alignment::Start)
        .align_y(alignment::Alignment::Center)
        .into()
    } else {
        Space::new().into()
    };

    let item = stack![
        container(content)
            .width(Length::Fill)
            .height(52.0)
            .padding(Padding {
                top: 8.0,
                // 右侧留给 scrollbar（约 10px）+ 视觉留白，避免 Alt+N 贴边。
                right: 22.0,
                bottom: 8.0,
                left: 14.0
            })
            .style(move |_t| container::Style {
                background: Some(Background::Color(if active {
                    ACCENT_BG
                } else {
                    Color::TRANSPARENT
                })),
                border: Border {
                    color: if active {
                        Color { a: 0.18, ..MARK }
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 1.0,
                    radius: border::radius(10.0),
                },
                ..container::Style::default()
            }),
        bar,
    ];

    // 鼠标：悬停选中、点击启动、右键菜单（css onMouseMove/onClick/onContextMenu）
    let area: MouseArea<'_, Message> = mouse_area(item)
        .on_enter(Message::HoverSelect(i))
        .on_press(Message::LaunchIndex(i))
        .on_right_press(Message::ContextMenu(i));
    area.into()
}

/// 结果列表滚动条：细圆角拇指、无轨道底，避免盖住右侧 Alt+N。
pub(crate) fn results_scrollbar() -> scrollable::Scrollbar {
    scrollable::Scrollbar::new()
        .width(8.0)
        .scroller_width(4.0)
        .margin(2.0)
}

/// 设置页共用同一套细滚动条外观。
pub(crate) fn results_scroll_style(
    _theme: &Theme,
    status: scrollable::Status,
) -> scrollable::Style {
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
        Color { a: 0.48, ..TEXT_MUTED }
    } else {
        Color { a: 0.22, ..TEXT_MUTED }
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
            background: Background::Color(Color { a: 0.92, ..BG_ELEVATED }),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: border::radius(999.0),
            },
            shadow: iced::Shadow {
                color: Color { a: 0.18, ..TEXT },
                offset: Vector::ZERO,
                blur_radius: 6.0,
            },
            icon: TEXT_MUTED,
        },
    }
}

/// 启用插件的短示例，用于占位/空态提示。
fn enabled_plugin_samples(state: &State) -> Vec<String> {
    let reg = state
        .plugin_registry
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::plugin::short_try_samples(&reg, 3)
}

fn search_placeholder(state: &State) -> String {
    let samples = enabled_plugin_samples(state);
    if samples.is_empty() {
        "搜索应用、网址，或直接输入关键词".to_string()
    } else {
        format!("搜索应用；或 {}", samples.join(" · "))
    }
}

/// 底部快捷键条（css .footer-bar）：kbd 胶囊 + 品牌位（快捷键标签）。
fn footer_bar(state: &State) -> Element<'static, Message> {
    fn pair(k: String, label: &'static str) -> Element<'static, Message> {
        row![
            container(text(k).size(10.0).color(TEXT))
                .padding([1.0, 5.0])
                .style(|_t| container::Style {
                    background: Some(Background::Color(BG_PANEL)),
                    border: Border {
                        color: BORDER,
                        width: 1.0,
                        radius: border::radius(4.0),
                    },
                    ..container::Style::default()
                }),
            text(label).size(11.0).color(TEXT_MUTED)
        ]
        .spacing(5.0)
        .align_y(alignment::Alignment::Center)
        .into()
    }
    let hotkey_label = if state.hotkey_label.is_empty() {
        "Alt + Space".to_string()
    } else {
        state.hotkey_label.clone()
    };
    let bar = row![
        pair("↑↓".into(), "选择"),
        pair("Enter".into(), "打开"),
        pair(
            crate::system::hotkey::display_label(&state.web_search_hotkey),
            "搜索"
        ),
        pair("Esc".into(), "关闭"),
        Space::new().width(Length::Fill),
        text(hotkey_label).size(11.0).color(TEXT_MUTED),
    ]
    .spacing(14.0)
    .align_y(alignment::Alignment::Center);

    container(bar)
        .width(Length::Fill)
        .padding([8.0, 16.0])
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_ELEVATED)),
            ..container::Style::default()
        })
        .into()
}

/// 右键菜单 overlay（css .ctx-menu：白底、1px 边框、radius 10、投影、项 hover 高亮）。
fn menu_overlay<'a>(
    state: &'a State,
    item: &'a crate::model::AppItem,
    x: f32,
    y: f32,
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
    if item.source != "everything-status" {
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
                .style(|_t, status| button::Style {
                    background: if status == button::Status::Hovered {
                        Some(Background::Color(Color { a: 0.14, ..MARK }))
                    } else {
                        None
                    },
                    text_color: if status == button::Status::Hovered {
                        MARK
                    } else {
                        TEXT
                    },
                    border: Border::default().rounded(7.0),
                    ..button::Style::default()
                }),
        );
    }

    let menu_box = container(col)
        .width(180.0)
        .padding(4.0)
        .style(|_t| container::Style {
            background: Some(Background::Color(BG_PANEL)),
            border: Border {
                color: BORDER,
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

/// 放大镜（SearchInput.tsx 里的 svg：circle r7 + 手柄线，stroke 1.8）。
struct Magnifier;

impl canvas::Program<Message> for Magnifier {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        // 注：tiny-skia 0.14 后端 stroke 几何实测不渲染（fill 正常），故用填充图元拼出放大镜
        let ring = iced::Point::new(9.0, 9.0);
        frame.fill(
            &canvas::Path::circle(ring, 7.0),
            Color::from_rgb8(0x6B, 0x72, 0x80),
        );
        frame.fill(
            &canvas::Path::circle(ring, 5.0),
            Color::from_rgb8(0xF7, 0xF8, 0xFA),
        );
        // 手柄：沿 45° 的细四边形（起点在圆环边上，向外延伸）
        let handle = canvas::Path::new(|p| {
            p.move_to(iced::Point::new(12.22, 13.78));
            p.line_to(iced::Point::new(16.62, 18.18));
            p.line_to(iced::Point::new(18.18, 16.62));
            p.line_to(iced::Point::new(13.78, 12.22));
            p.close();
        });
        frame.fill(&handle, Color::from_rgb8(0x6B, 0x72, 0x80));
        vec![frame.into_geometry()]
    }
}

fn first_char(s: &str) -> String {
    s.chars().next().map(|c| c.to_string()).unwrap_or_default()
}

/// 路径过长时中间省略，保留首尾可辨识片段。
fn truncate_middle(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    let keep_head = max_chars.saturating_sub(1) / 2;
    let keep_tail = max_chars.saturating_sub(1) - keep_head;
    let head: String = chars[..keep_head].iter().collect();
    let tail: String = chars[chars.len() - keep_tail..].iter().collect();
    format!("{head}…{tail}")
}

// container/text_input 等类型仅用于签名约束的引用，避免未使用告警
#[allow(unused_imports)]
use iced::widget::{Container, TextInput as _};
