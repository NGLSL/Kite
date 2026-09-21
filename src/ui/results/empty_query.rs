//! 空 Query 默认列表组装。
//!
//! 列表顺序服务 UI 仪表盘分区（`search_view` 按 `grid_recent_count` 切开）：
//! **最近使用区在前、固定区在后**。这与检索 API 空 Query 的「固定优先」产品语义
//! 不是同一层——领域词见 `docs/CONTEXT.md` 的 Pin 条目。

use std::collections::HashSet;

use crate::model::{AppItem, SearchResult};

/// 空 Query 展示列表。
pub(crate) struct EmptyQueryLists {
    /// 最近使用 + 默认补齐（不含已固定）。
    pub recent: Vec<SearchResult>,
    /// 用户固定项（最多 8），顺序与 `pinned_ids`（pinned_at 降序）一致。
    pub pinned: Vec<SearchResult>,
}

/// 组装空 Query 默认列表。
///
/// - 最近：真实启动历史最多 16 条，排除已固定
/// - 补齐：索引中其它可见应用，仍排除已固定
/// - 固定：按 `pinned_ids` 给定顺序（存储层 `pinned_at DESC`）取前 8 个仍在索引中的项；
///   超出上限时**入选集合确定**，不依赖 HashSet 迭代顺序
pub(crate) fn build_empty_query_lists(
    recent_ids: &[String],
    pinned_ids: &[String],
    index_apps: &[AppItem],
) -> EmptyQueryLists {
    let pinned_set: HashSet<&str> = pinned_ids.iter().map(String::as_str).collect();

    let mut recent: Vec<SearchResult> = Vec::new();
    for id in recent_ids {
        if pinned_set.contains(id.as_str()) {
            continue;
        }
        if recent.iter().any(|h| h.item.id == *id) {
            continue;
        }
        if let Some(app) = index_apps.iter().find(|a| &a.id == id) {
            recent.push(SearchResult::scored(app.clone(), 1, "recent"));
        }
        if recent.len() >= 16 {
            break;
        }
    }

    for app in index_apps {
        if recent.len() >= 16 {
            break;
        }
        if pinned_set.contains(app.id.as_str()) || recent.iter().any(|h| h.item.id == app.id) {
            continue;
        }
        if crate::model::is_hidden_on_empty_fill(&app.source, &app.target) {
            continue;
        }
        recent.push(SearchResult::scored(app.clone(), 0, "default"));
    }

    let mut pinned_items: Vec<SearchResult> = Vec::new();
    for id in pinned_ids {
        if pinned_items.len() >= 8 {
            break;
        }
        if let Some(app) = index_apps.iter().find(|a| &a.id == id) {
            pinned_items.push(SearchResult::scored(app.clone(), 2, "pinned"));
        }
    }

    EmptyQueryLists {
        recent,
        pinned: pinned_items,
    }
}
