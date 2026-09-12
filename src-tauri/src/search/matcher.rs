//! 多路召回：对同一 Query 收集候选，每条只保留最高分与来源。

use std::borrow::Cow;

use crate::model::{AppItem, SearchResult};
use crate::search::alias;
use crate::search::fuzzy::fuzzy_match;
use crate::search::ranker::{
    SCORE_BUILTIN_ALIAS_EXACT, SCORE_NAME_EXACT, SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL,
    SCORE_PREFIX, SCORE_SUBSTRING, SCORE_USER_ALIAS_EXACT,
};

/// 预计算字段为空时退回现场规范化。
fn cow_normalized<'a>(normalized: &'a str, raw: &'a str) -> Cow<'a, str> {
    if normalized.is_empty() {
        Cow::Owned(crate::search::normalizer::normalize_name(raw))
    } else {
        Cow::Borrowed(normalized)
    }
}

fn take_best(
    best: Option<(i32, &'static str)>,
    score: i32,
    kind: &'static str,
) -> Option<(i32, &'static str)> {
    Some(match best {
        Some((bs, bk)) if bs >= score => (bs, bk),
        _ => (score, kind),
    })
}

/// `user_targets`：用户 Alias 指向的应用名（已小写）。
pub fn collect_candidates(apps: &[AppItem], q: &str, user_targets: &[String]) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for item in apps {
        if let Some(hit) = match_item(item, q, user_targets) {
            out.push(hit);
        }
    }
    out
}

fn match_item(item: &AppItem, q: &str, user_targets: &[String]) -> Option<SearchResult> {
    // 预计算字段直接借用；缺省时才临时规范化（Cow 避免每键全量分配）
    let name = cow_normalized(item.normalized_name.as_str(), &item.name);
    let display = cow_normalized(item.normalized_display.as_str(), &item.display_name);
    // 内置 Alias 每 Query 只解析一次（单表数据源）
    let alias_targets = alias::targets_for(q);

    let mut best: Option<(i32, &'static str)> = None;

    // 0) 用户 Alias（优先级最高）
    let hit_user = user_targets
        .iter()
        .any(|t| name.as_ref() == t.as_str() || name.contains(t.as_str()) || display.contains(t.as_str()));
    if hit_user {
        best = take_best(best, SCORE_USER_ALIAS_EXACT, "user-alias-exact");
    }

    // 1) 内置 Alias
    if let Some(fragments) = alias_targets {
        if fragments
            .iter()
            .any(|t| name.contains(t) || display.contains(t))
        {
            best = take_best(best, SCORE_BUILTIN_ALIAS_EXACT, "alias-exact");
        }
    }

    // 2) 名称 Exact / Prefix / Substring
    for candidate in [name.as_ref(), display.as_ref()] {
        if candidate == q {
            best = take_best(best, SCORE_NAME_EXACT, "exact");
        } else if candidate.starts_with(q) {
            best = take_best(best, SCORE_PREFIX, "prefix");
        } else if candidate.contains(q) {
            best = take_best(best, SCORE_SUBSTRING, "substring");
        }
    }

    // 3) 拼音
    if !item.pinyin.is_empty() {
        if item.pinyin == q {
            best = take_best(best, SCORE_PINYIN_EXACT, "pinyin-exact");
        } else if item.pinyin.starts_with(q) {
            best = take_best(best, SCORE_PINYIN_EXACT - 50, "pinyin-prefix");
        }
    }
    if !item.pinyin_initials.is_empty() {
        if item.pinyin_initials == q {
            best = take_best(best, SCORE_PINYIN_INITIAL, "pinyin-initial");
        } else if q.len() >= 2 && item.pinyin_initials.starts_with(q) {
            best = take_best(best, SCORE_PINYIN_INITIAL - 40, "pinyin-initial-prefix");
        }
    }

    // 4) Fuzzy 兜底
    if best.is_none() {
        if let Some((score, _)) = fuzzy_name_or_tokens(q, &name).or_else(|| fuzzy_name_or_tokens(q, &display)) {
            best = take_best(best, score, "fuzzy");
        }
    }

    let (score, matched_by) = best?;
    Some(SearchResult {
        item: item.clone(),
        score,
        matched_by: matched_by.to_string(),
    })
}

fn fuzzy_name_or_tokens(q: &str, name: &str) -> Option<(i32, usize)> {
    if let Some(hit) = fuzzy_match(q, name) {
        return Some(hit);
    }
    name.split_whitespace()
        .filter_map(|tok| fuzzy_match(q, tok))
        .max_by_key(|(score, _)| *score)
}
