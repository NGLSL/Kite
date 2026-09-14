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
use std::collections::HashSet;
use std::path::Path;

// App Paths 只有 exe 文件名。若同一安装目录已有可搜索的开始菜单/桌面入口，
// 让这个更易辨认的入口排在前面，但保留原始 exe 供用户直接启动。
const RAW_APP_PATH_DISCOUNT: i32 = SCORE_NAME_EXACT - SCORE_PREFIX + 1;

pub fn prefer_friendly_install_entries(hits: &mut [SearchResult]) {
    let friendly_dirs: HashSet<String> = hits
        .iter()
        .filter(|h| matches!(h.item.source.as_str(), "start-menu" | "desktop"))
        .filter_map(|h| Path::new(&h.item.target).parent())
        // 不将 Program Files 等公共父目录误判为同一款应用。
        .filter(|dir| dir.components().count() >= 4)
        .map(|dir| {
            let normalized = normalize_windows_path(&dir.to_string_lossy());
            format!("{}\\", normalized.trim_end_matches('\\'))
        })
        .collect();
    for hit in hits.iter_mut().filter(|h| h.item.source == "app-paths") {
        let target = normalize_windows_path(&hit.item.target);
        if target
            .match_indices('\\')
            .any(|(end, _)| friendly_dirs.contains(&target[..=end]))
        {
            hit.score -= RAW_APP_PATH_DISCOUNT;
        }
    }
}

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
