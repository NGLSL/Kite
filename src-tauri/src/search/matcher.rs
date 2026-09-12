//! 多路召回：对同一 Query 收集候选，每条只保留最高分与来源。

use crate::model::{AppItem, SearchResult};
use crate::search::alias;
use crate::search::fuzzy::fuzzy_match;
use crate::search::ranker::{
    SCORE_BUILTIN_ALIAS_EXACT, SCORE_NAME_EXACT, SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL,
    SCORE_PREFIX, SCORE_SUBSTRING, SCORE_USER_ALIAS_EXACT,
};

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
    let name = if item.normalized_name.is_empty() {
        crate::search::normalizer::normalize_name(&item.name)
    } else {
        item.normalized_name.clone()
    };
    let display = crate::search::normalizer::normalize_name(&item.display_name);

    let mut best: Option<(i32, &'static str)> = None;

    // 0) 用户 Alias（优先级最高）
    let hit_user = user_targets
        .iter()
        .any(|t| name == *t || name.contains(t.as_str()) || display.contains(t.as_str()));
    if hit_user {
        best = take_best(best, SCORE_USER_ALIAS_EXACT, "user-alias-exact");
    }

    // 1) 内置 Alias
    if alias::is_builtin_alias(q)
        && (alias::alias_targets_name(q, &name) || alias::alias_targets_name(q, &display))
    {
        best = take_best(best, SCORE_BUILTIN_ALIAS_EXACT, "alias-exact");
    }

    // 2) 名称 Exact / Prefix / Substring
    for candidate in [&name, &display] {
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
