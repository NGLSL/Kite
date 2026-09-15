//! 搜索管线：规范化 → 多路召回 → 统一评分 → Top N。

mod alias;
#[cfg(test)]
mod bench;
#[cfg(test)]
mod eval;
mod fuzzy;
mod lists;
mod matcher;
mod normalizer;
mod pinyin;
pub mod ranker;
pub mod retrieval;
#[cfg(test)]
mod tests;
pub mod url;

pub use lists::{name_candidates, order_by_recent, rerank};
pub use matcher::UserTarget;
pub use retrieval::RetrievalIndex;

use crate::model::{AppItem, SearchResult};

/// 默认返回条数（首页）。
pub const TOP_N: usize = 10;

/// 排序结果缓存/返回上限；滚动加载最多翻到这里。
pub const MAX_RESULTS: usize = 200;

/// 索引用名称规范化（供 AppItem 预计算）。
pub fn normalize_for_index(name: &str) -> String {
    normalizer::normalize_name(name)
}

/// 索引用拼音预计算：返回 (全拼无空格, 首字母)。
pub fn pinyin_of(text: &str) -> (String, String) {
    pinyin::precompute(text)
}

/// 入口：空 Query 给默认列表，否则索引多路召回 + 排序。
/// `user_alias_targets`：用户 Alias 目标（稳定 id / 名称）。
/// `max_results`：返回条数上限（滚动加载时调用方逐步放大；截断在 IPC 边界做）。
pub fn search(
    apps: &[AppItem],
    query: &str,
    user_alias_targets: &[UserTarget],
    max_results: usize,
) -> Vec<SearchResult> {
    let q = normalizer::normalize_query(query);
    if q.is_empty() {
        // 无历史时退回索引顺序；有历史请调用方走 order_by_recent
        return apps
            .iter()
            .take(max_results)
            .map(|item| SearchResult::scored(item.clone(), 0, "default"))
            .collect();
    }

    // 与系统入口共用检索内核；此处 apps 为应用快照（系统入口由 search_with_system 传入）
    retrieval::search_with_snapshot(apps, &[], query, user_alias_targets, max_results)
}

/// 应用 + 系统入口统一召回（UI 热路径）。
pub fn search_with_system(
    apps: &[AppItem],
    system_entries: &[AppItem],
    query: &str,
    user_alias_targets: &[UserTarget],
    max_results: usize,
) -> Vec<SearchResult> {
    let q = normalizer::normalize_query(query);
    if q.is_empty() {
        return search(apps, query, user_alias_targets, max_results);
    }
    retrieval::search_with_snapshot(apps, system_entries, query, user_alias_targets, max_results)
}

/// 使用预构建索引搜索（快照同代，避免每键重建）。
pub fn search_with_index(
    index: &RetrievalIndex,
    query: &str,
    user_alias_targets: &[UserTarget],
    max_results: usize,
) -> Vec<SearchResult> {
    index.search(query, user_alias_targets, max_results)
}

/// 统一最终排序入口：多路召回 → 验证 → 个性化（截断前）→ 一次截断。
/// UI 与系统入口主路径应走这里，不再「先截断再加分」或层排序后再纯分数重排。
pub fn search_with_personalization(
    index: &RetrievalIndex,
    query: &str,
    user_alias_targets: &[UserTarget],
    personalization: Option<&crate::history::Personalization>,
    max_results: usize,
) -> Vec<SearchResult> {
    index.search_personalized(query, user_alias_targets, personalization, max_results)
}

/// 无预构建索引时的快照路径（测试与冷启动）。
pub fn search_system_personalized(
    apps: &[AppItem],
    system_entries: &[AppItem],
    query: &str,
    user_alias_targets: &[UserTarget],
    personalization: Option<&crate::history::Personalization>,
    max_results: usize,
) -> Vec<SearchResult> {
    let index = RetrievalIndex::build(apps, system_entries);
    index.search_personalized(query, user_alias_targets, personalization, max_results)
}
