//! 统一评分。所有分数集中在这里，禁止散落到 matcher。

/// 用户 Alias（Phase 3+；常量先占位）。
pub const SCORE_USER_ALIAS_EXACT: i32 = 1100;
pub const SCORE_NAME_EXACT: i32 = 1000;
/// 连写精确（todo ↔ To Do）；低于真实名称精确，高于普通前缀。
pub const SCORE_COMPACT_EXACT: i32 = 920;
/// 有序多词命中（visual code / vs code）。
pub const SCORE_TOKEN_SEQ: i32 = 880;
pub const SCORE_BUILTIN_ALIAS_EXACT: i32 = 950;
pub const SCORE_WORD_EXACT: i32 = 850;
pub const SCORE_PREFIX: i32 = 800;
/// 连写包含（限制短 Query 误召回，见 matcher 最小长度）。
pub const SCORE_COMPACT_SUBSTRING: i32 = 780;
pub const SCORE_PINYIN_EXACT: i32 = 750;
pub const SCORE_PINYIN_INITIAL: i32 = 700;
/// 拼音首字母中间命中（kz ← 键盘控制 / 向日葵远程控制）；低于前缀式首字母。
pub const SCORE_PINYIN_INITIAL_INNER: i32 = 620;
/// 可信别名的拼音前缀比名称的宽松音节匹配更明确。
pub const SCORE_KEYWORD_PINYIN_EXACT: i32 = 780;
pub const SCORE_KEYWORD_PINYIN_PREFIX: i32 = 760;
pub const SCORE_KEYWORD_PINYIN_INNER: i32 = 600;
/// 单词前缀（含 Camel 拆词后的 terminal ← ter）。
pub const SCORE_WORD_PREFIX: i32 = 720;
/// 英文多词首字母缩写（ndm → Neat Download Manager）；低于真实词匹配。
pub const SCORE_ACRONYM: i32 = 680;
pub const SCORE_SUBSTRING: i32 = 550;
/// Fuzzy 上限；具体分由编辑距离映射。
pub const SCORE_FUZZY_MAX: i32 = 450;
/// Windows 标准搜索资源可扩大召回，但不能与真实名称和明确别名同权。
pub const CONTEXT_DISCOUNT: i32 = 160;
pub const CONTEXT_COMPACT_DISCOUNT: i32 = 220;
pub const CONTEXT_PINYIN_DISCOUNT: i32 = 130;

use crate::model::SearchResult;
use std::collections::HashSet;
use std::path::Path;

pub(crate) fn friendly_dirs_from<'a>(
    items: impl Iterator<Item = (&'a str, &'a String)>,
) -> HashSet<String> {
    items
        .filter(|(source, _)| matches!(*source, "start-menu" | "desktop"))
        .filter_map(|(_, target)| Path::new(target).parent())
        // 不将 Program Files 等公共父目录误判为同一款应用。
        .filter(|dir| dir.components().count() >= 4)
        .map(|dir| {
            let normalized = normalize_windows_path(&dir.to_string_lossy());
            format!("{}\\", normalized.trim_end_matches('\\'))
        })
        .collect()
}

pub(crate) fn hit_under_friendly_dir(target: &str, dirs: &HashSet<String>) -> bool {
    let target = normalize_windows_path(target);
    target
        .match_indices('\\')
        .any(|(end, _)| dirs.contains(&target[..=end]))
}

pub fn rank_and_truncate(hits: Vec<SearchResult>, top_n: usize) -> Vec<SearchResult> {
    // 先生成小写键再排序：比较器里不做 to_lowercase，避免 O(n log n) 次分配
    let mut keyed: Vec<(i32, i32, usize, String, String, SearchResult)> = hits
        .into_iter()
        .map(|h| {
            let tier = if h.quality_tier != 0 {
                h.quality_tier
            } else {
                crate::history::quality_tier(h.score)
            };
            let lower = h.item.name.to_lowercase();
            let len = h.item.name.len();
            let id = h.item.id.clone();
            (tier, h.score, len, lower, id, h)
        })
        .collect();
    keyed.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            // 同分：短名优先（更精确）
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.4.cmp(&b.4))
    });
    keyed.truncate(top_n);
    keyed.into_iter().map(|(_, _, _, _, _, h)| h).collect()
}

fn normalize_windows_path(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppItem;

    fn tied_hit(id: &str) -> SearchResult {
        SearchResult::scored(
            AppItem::scanned(
                id.to_string(),
                "Same name".to_string(),
                format!(r"C:\{id}.exe"),
                None,
                None,
                "start-menu",
            ),
            500,
            "test",
        )
    }

    #[test]
    fn equal_scores_have_a_stable_order_independent_of_input_order() {
        let forward = rank_and_truncate(vec![tied_hit("b"), tied_hit("a")], 10);
        let reverse = rank_and_truncate(vec![tied_hit("a"), tied_hit("b")], 10);
        let ids = |hits: &[SearchResult]| {
            hits.iter()
                .map(|hit| hit.item.id.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(ids(&forward), ids(&reverse));
        assert_eq!(ids(&forward), vec!["a".to_string(), "b".to_string()]);
    }
}
