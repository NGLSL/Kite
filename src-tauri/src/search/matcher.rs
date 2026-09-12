//! Phase 1 匹配：仅 Exact 与 Prefix。

use crate::model::AppItem;

use super::{SCORE_EXACT, SCORE_PREFIX};

pub fn normalize_query(q: &str) -> String {
    q.trim().to_lowercase()
}

fn normalize_name(n: &str) -> String {
    n.trim().to_lowercase()
}

/// 单条应用的最佳匹配；无命中返回 None。返回 (score, matched_by)。
pub fn match_item(item: &AppItem, q: &str) -> Option<(i32, &'static str)> {
    let name = normalize_name(&item.name);
    let display = normalize_name(&item.display_name);

    let mut best: Option<(i32, &'static str)> = None;
    for candidate in [&name, &display] {
        let (score, kind) = if candidate == q {
            (SCORE_EXACT, "exact")
        } else if candidate.starts_with(q) {
            (SCORE_PREFIX, "prefix")
        } else {
            continue;
        };
        best = Some(match best {
            Some((bs, bk)) if bs >= score => (bs, bk),
            _ => (score, kind),
        });
    }
    best
}
