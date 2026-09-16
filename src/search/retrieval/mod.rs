//! 索引驱动的多路召回内核。
//!
//! 可搜索快照（应用 + 系统入口）与 RetrievalIndex 同代只读。
//! Query 解析一次，各通道独立召回取并集，再逐候选验证并统一排序。

mod channels;
mod doc;
pub(crate) mod query;
mod symspell;
pub(crate) mod verify;

pub use doc::{IndexedDoc, RetrievalIndex, StructureStats};
pub use query::{ChannelStats, ParsedQuery};
pub use verify::{reference_search, MatcherScratch, QueryContext, RankedHit, SCORE_SKIP};

use crate::model::{AppItem, SearchResult};
use crate::search::matcher::UserTarget;

/// 一次检索执行过程中的协作取消与观察点。
///
/// `cancelled` 在阶段边界与候选验证批次上被查询；一旦为真，整次检索作废——
/// 不返回部分结果、不写缓存、不回调。
///
/// `on_verify_start` 只在进入候选验证前触发一次：取消测试用它把「旧任务正在验证」
/// 固定成确定的时刻，而不是靠 sleep 赌调度。
#[derive(Clone, Copy)]
pub struct SearchRun<'a> {
    cancelled: &'a dyn Fn() -> bool,
    on_verify_start: Option<&'a dyn Fn()>,
}

impl<'a> SearchRun<'a> {
    /// 普通执行：只带取消检查。
    pub fn new(cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            cancelled,
            on_verify_start: None,
        }
    }

    /// 带「即将进入验证」观察点（常驻 worker 为取消测试提供）。
    pub fn with_verify_hook(cancelled: &'a dyn Fn() -> bool, on_verify_start: &'a dyn Fn()) -> Self {
        Self {
            cancelled,
            on_verify_start: Some(on_verify_start),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        (self.cancelled)()
    }

    /// 进入候选验证前触发观察点。
    pub fn entering_verify(&self) {
        if let Some(hook) = self.on_verify_start {
            hook();
        }
    }
}

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
