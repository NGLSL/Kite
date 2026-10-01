//! 空 Query 列表与 Alias 选择器等辅助入口，不参与多路召回评分。

use std::collections::HashSet;

use crate::model::{AppItem, SearchResult};

use super::normalizer;
use super::ranker;
/// 空 Query 默认列表：固定项优先，其次最近使用，不足再按索引顺序补满。
/// Tier C（命令 Alias、App Paths/Uninstall）默认不进补满段；Pin / 最近使用仍可出现。
/// 用户 Portable 属 Tier A，会进补满段。
pub fn order_by_recent(
    apps: &[AppItem],
    recent_ids: &[String],
    pinned_ids: &[String],
    top_n: usize,
) -> Vec<SearchResult> {
    order_by_recent_with_hidden(apps, recent_ids, pinned_ids, &HashSet::new(), top_n)
}

/// 空 Query 列表的隐藏感知入口，供 UI 实际组装最近/固定/补满区时使用。
/// 隐藏项在三段中都不占名额；未隐藏项仍按原有固定、最近、默认顺序补齐。
pub fn order_by_recent_with_hidden(
    apps: &[AppItem],
    recent_ids: &[String],
    pinned_ids: &[String],
    hidden_ids: &HashSet<String>,
    top_n: usize,
) -> Vec<SearchResult> {
    let has_hidden = !hidden_ids.is_empty();
    let mut hits: Vec<SearchResult> = Vec::with_capacity(top_n.min(apps.len()));
    let push_hit =
        |id: &str, score: i32, matched_by: &'static str, hits: &mut Vec<SearchResult>| -> bool {
            if hits.iter().any(|h| h.item.id == id) {
                return false;
            }
            if let Some(item) = apps.iter().find(|a| a.id == id) {
                if has_hidden
                    && hidden_ids.contains(id)
                    && crate::model::is_hideable_application_source(&item.source)
                {
                    return false;
                }
                hits.push(SearchResult::scored(item.clone(), score, matched_by));
                return true;
            }
            false
        };
    for id in pinned_ids {
        if hits.len() >= top_n {
            break;
        }
        push_hit(id, 2, "pinned", &mut hits);
    }
    for id in recent_ids {
        if hits.len() >= top_n {
            break;
        }
        push_hit(id, 1, "recent", &mut hits);
    }
    for item in apps {
        if hits.len() >= top_n {
            break;
        }
        if hits.iter().any(|h| h.item.id == item.id) {
            continue;
        }
        if has_hidden
            && hidden_ids.contains(&item.id)
            && crate::model::is_hideable_application_source(&item.source)
        {
            continue;
        }
        if crate::model::is_hidden_on_empty_fill(&item.source, &item.target) {
            continue;
        }
        hits.push(SearchResult::scored(item.clone(), 0, "default"));
    }
    hits
}

/// Alias 目标选择器用：按名称/拼音从索引挑候选，Top N。
/// 轻量实现（前缀 > 包含，短名优先），不走完整评分管线。
pub fn name_candidates(apps: &[AppItem], query: &str, top_n: usize) -> Vec<SearchResult> {
    name_candidates_with_hidden(apps, query, &HashSet::new(), top_n)
}

/// Alias 目标选择器的隐藏感知入口；隐藏项保留在原 Alias 存储中，但不再作为新目标候选。
pub fn name_candidates_with_hidden(
    apps: &[AppItem],
    query: &str,
    hidden_ids: &HashSet<String>,
    top_n: usize,
) -> Vec<SearchResult> {
    fn norm<'a>(precomputed: &'a str, raw: &'a str) -> std::borrow::Cow<'a, str> {
        if precomputed.is_empty() {
            std::borrow::Cow::Owned(normalizer::normalize_name(raw))
        } else {
            std::borrow::Cow::Borrowed(precomputed)
        }
    }

    let q = normalizer::normalize_query(query);
    if q.is_empty() {
        return Vec::new();
    }
    let has_hidden = !hidden_ids.is_empty();
    let mut hits: Vec<SearchResult> = Vec::new();
    for item in apps {
        if has_hidden
            && hidden_ids.contains(&item.id)
            && crate::model::is_hideable_application_source(&item.source)
        {
            continue;
        }
        let name = norm(&item.normalized_name, &item.name);
        let display = norm(&item.normalized_display, &item.display_name);
        let mut score = 0i32;
        if name == q || display == q {
            score = 4;
        } else if name.starts_with(&*q) || display.starts_with(&*q) {
            score = 3;
        } else if !item.pinyin.is_empty() && item.pinyin.starts_with(&*q) {
            score = 2;
        } else if name.contains(&*q) || display.contains(&*q) {
            score = 1;
        } else if !item.pinyin_initials.is_empty() && item.pinyin_initials.starts_with(&*q) {
            score = 1;
        }
        if score > 0 {
            hits.push(SearchResult::scored(item.clone(), score, "candidate"));
        }
    }
    ranker::rank_and_truncate(hits, top_n)
}

/// 历史加分后重新排序截断（commands 在改分后调用）。
pub fn rerank(hits: Vec<SearchResult>, top_n: usize) -> Vec<SearchResult> {
    ranker::rank_and_truncate(hits, top_n)
}
