//! Kite 主界面：高质感双态横向布局（zTools 网格仪表盘 + 双列搜索卡片流）与主题渲染。
//! 支持深色模式（#0F1115）、浅色模式（#FFFFFF）以及跟随系统。

use iced::widget::{column, container, space::Space, stack, Id as WidgetId};
use iced::{border, Background, Border, Element, Length};

use super::theme::ThemeTokens;
use super::{Message, State};

mod input;
mod overlays;
mod panel;
mod results;
mod text;

pub(crate) use results::{results_scroll_style_tokens, results_scrollbar};
#[cfg(test)]
pub(super) use text::truncate_display_label;

/// 行步进：双列卡片高 58 + 间距 6 = 64
pub const ROW_STEP: f32 = 64.0;
pub const SCROLL_ID: &str = "poc-results";
const INPUT_ID: &str = "poc-input";

pub fn input_id() -> WidgetId {
    WidgetId::new(INPUT_ID)
}

pub fn scroll_id() -> WidgetId {
    WidgetId::new(SCROLL_ID)
}

pub fn view(state: &State) -> Element<'_, Message> {
    let tokens = state.theme_tokens();

    // 窗口外框：1px 细微光边框，去除割裂横线，自然空气留白
    let panel = column![
        input::search_row(state, tokens),
        results::results_area(state, tokens),
        overlays::footer_bar(state, tokens),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let root = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(tokens.bg_window)),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(0.0),
            },
            ..container::Style::default()
        });

    // 右键菜单以 overlay 叠加；无菜单时 overlay 为空，保持 Scrollable 树结构稳定
    let root: Element<'_, Message> = root.into();
    let overlay: Element<'_, Message> = match &state.menu {
        Some((item, x, y)) => overlays::menu_overlay(state, item, *x, *y, tokens),
        None => Space::new().width(Length::Fill).height(Length::Fill).into(),
    };
    let content: Element<'_, Message> = stack![root, overlay].into();

    // 快捷提示 Toast（复制成功、状态通知等）
    if let Some(msg) = &state.flash {
        stack![content, overlays::toast(msg, tokens)].into()
    } else {
        content
    }
}

fn divider(tokens: ThemeTokens) -> Element<'static, Message> {
    let border_color = tokens.border_subtle;
    container(Space::new().height(1.0))
        .width(Length::Fill)
        .height(1.0)
        .style(move |_t| container::Style {
            background: Some(Background::Color(border_color)),
            ..container::Style::default()
        })
        .into()
}

pub(crate) struct ProviderMeta {
    pub chip_label: &'static str,
    pub placeholder: &'static str,
}

pub(crate) fn provider_meta(provider_id: &str) -> ProviderMeta {
    match provider_id {
        "calculate" => ProviderMeta {
            chip_label: "🧮 计算器",
            placeholder: "输入算式直接计算，Enter 复制结果",
        },
        "windows" => ProviderMeta {
            chip_label: "⊞ 窗口",
            placeholder: "输入关键字过滤窗口，Enter 激活",
        },
        "ts" => ProviderMeta {
            chip_label: "⏱ 时间戳",
            placeholder: "输入时间戳或日期转换",
        },
        "uuid" => ProviderMeta {
            chip_label: "🔑 UUID",
            placeholder: "生成 UUID，Enter 复制",
        },
        "hash" => ProviderMeta {
            chip_label: "# 哈希",
            placeholder: "输入文本计算哈希",
        },
        "json" => ProviderMeta {
            chip_label: "{ } JSON",
            placeholder: "输入 JSON 字符串，Enter 打开格式化窗口",
        },
        _ => ProviderMeta {
            chip_label: "插件",
            placeholder: "插件模式中，Enter 执行…",
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

#[cfg(test)]
mod tests;
