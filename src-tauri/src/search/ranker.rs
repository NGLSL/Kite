//! 统一评分。所有分数集中在这里，禁止散落到 matcher。

/// 用户 Alias（Phase 3+；常量先占位）。
#[allow(dead_code)]
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

pub fn rank_and_truncate(mut hits: Vec<SearchResult>, top_n: usize) -> Vec<SearchResult> {
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            // 同分：短名优先（更精确）
            .then_with(|| a.item.name.len().cmp(&b.item.name.len()))
            .then_with(|| a.item.name.to_lowercase().cmp(&b.item.name.to_lowercase()))
    });
    hits.truncate(top_n);
    hits
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
