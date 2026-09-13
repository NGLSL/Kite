//! 统一评分。所有分数集中在这里，禁止散落到 matcher。

/// 用户 Alias（Phase 3+；常量先占位）。
pub const SCORE_USER_ALIAS_EXACT: i32 = 1100;
pub const SCORE_NAME_EXACT: i32 = 1000;
pub const SCORE_BUILTIN_ALIAS_EXACT: i32 = 950;
pub const SCORE_PREFIX: i32 = 800;
pub const SCORE_PINYIN_EXACT: i32 = 750;
pub const SCORE_PINYIN_INITIAL: i32 = 700;
pub const SCORE_SUBSTRING: i32 = 550;
/// Fuzzy 上限；具体分由编辑距离映射。
pub const SCORE_FUZZY_MAX: i32 = 450;

use crate::model::SearchResult;

pub fn rank_and_truncate(hits: Vec<SearchResult>, top_n: usize) -> Vec<SearchResult> {
    // 先生成小写键再排序：比较器里不做 to_lowercase，避免 O(n log n) 次分配
    let mut keyed: Vec<(i32, usize, String, SearchResult)> = hits
        .into_iter()
        .map(|h| {
            let lower = h.item.name.to_lowercase();
            let len = h.item.name.len();
            (h.score, len, lower, h)
        })
        .collect();
    keyed.sort_by(|a, b| {
        b.0.cmp(&a.0)
            // 同分：短名优先（更精确）
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    keyed.truncate(top_n);
    keyed.into_iter().map(|(_, _, _, h)| h).collect()
}

/// 编辑距离 → fuzzy 分数。
pub fn fuzzy_score(distance: usize) -> i32 {
    match distance {
        0 => SCORE_NAME_EXACT,
        1 => 400,
        2 => 280,
        _ => 0,
    }
}
