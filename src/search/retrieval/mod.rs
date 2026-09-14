//! 索引驱动的多路召回内核。
//!
//! 可搜索快照（应用 + 系统入口）与 RetrievalIndex 同代只读。
//! Query 解析一次，各通道独立召回取并集，再逐候选验证并统一排序。

mod channels;
mod doc;
pub(crate) mod query;
mod symspell;
pub(crate) mod verify;

pub use doc::{IndexedDoc, RetrievalIndex};
pub use query::{ChannelStats, ParsedQuery};
pub use verify::{reference_search, SCORE_SKIP};

use crate::model::{AppItem, SearchResult};
use crate::search::matcher::UserTarget;

/// 用应用列表构建临时索引并搜索（测试与无缓存路径）。
pub fn search_with_snapshot(
    apps: &[AppItem],
    system_entries: &[AppItem],
    query: &str,
    user_targets: &[UserTarget],
    max_results: usize,
) -> Vec<SearchResult> {
    let index = RetrievalIndex::build(apps, system_entries);
    index.search(query, user_targets, max_results)
}
