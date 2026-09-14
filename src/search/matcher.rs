//! 多路召回：对同一 Query 收集候选，每条只保留最高分与来源。

use std::borrow::Cow;

use crate::model::{AppItem, SearchResult};
use crate::search::alias;
use crate::search::fuzzy::fuzzy_match;
use crate::search::normalizer::{compact, split_camel, tokens};
use crate::search::ranker::{
    SCORE_BUILTIN_ALIAS_EXACT, SCORE_COMPACT_EXACT, SCORE_COMPACT_SUBSTRING, SCORE_NAME_EXACT,
    SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL, SCORE_PREFIX, SCORE_SUBSTRING, SCORE_TOKEN_SEQ,
    SCORE_USER_ALIAS_EXACT, SCORE_WORD_EXACT, SCORE_WORD_PREFIX,
};

/// 连写包含最短 Query 长度：避免 `to` 等短片段误召回大量应用。
const MIN_COMPACT_SUBSTR_LEN: usize = 3;
/// 单词前缀最短 Query 长度。
const MIN_WORD_PREFIX_LEN: usize = 2;

/// 用户 Alias 的一行匹配素材：优先按稳定 id 命中，旧行（无 id）退回名称包含。
pub struct UserTarget {
    pub id: Option<String>,
    pub name: String,
}

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

/// `user_targets`：用户 Alias 的目标（已小写名称 / 可选稳定 id）。
pub fn collect_candidates(apps: &[AppItem], q: &str, user_targets: &[UserTarget]) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for item in apps {
        if let Some(hit) = match_item(item, q, user_targets) {
            out.push(hit);
        }
    }
    out
}

fn match_item(item: &AppItem, q: &str, user_targets: &[UserTarget]) -> Option<SearchResult> {
    // 预计算字段直接借用；缺省时才临时规范化（Cow 避免每键全量分配）
    let name = cow_normalized(item.normalized_name.as_str(), &item.name);
    let display = cow_normalized(item.normalized_display.as_str(), &item.display_name);
    let q_compact = compact(q);
    // 内置 Alias 每 Query 只解析一次（单表数据源）
    let alias_targets = alias::targets_for(q);

    let mut best: Option<(i32, &'static str)> = None;

    // 0) 用户 Alias（优先级最高）：有稳定 id 的按 id 精确点名，避免重名误绑
    let hit_user = user_targets.iter().any(|t| match &t.id {
        Some(id) => item.id == *id,
        None => {
            let n = t.name.to_lowercase();
            name.as_ref() == n.as_str() || name.contains(n.as_str()) || display.contains(n.as_str())
        }
    });
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

    // 2b) Compact：todo ↔ To Do / Microsoft To Do
    if q_compact.len() >= 2 {
        for candidate in [name.as_ref(), display.as_ref()] {
            let c = compact(candidate);
            if c == q_compact {
                best = take_best(best, SCORE_COMPACT_EXACT, "compact-exact");
            } else if q_compact.len() >= MIN_COMPACT_SUBSTR_LEN && c.contains(&q_compact) {
                best = take_best(best, SCORE_COMPACT_SUBSTRING, "compact-substring");
            }
        }
    }

    // 2c) 词边界：exact / prefix，以及 Camel 拆词（XTerminal → terminal）
    let name_tokens: Vec<String> = {
        let mut t: Vec<String> = tokens(name.as_ref())
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        for raw in [&item.name, &item.display_name] {
            for part in split_camel(raw) {
                if !t.contains(&part) {
                    t.push(part);
                }
            }
        }
        t
    };
    for tok in &name_tokens {
        if tok == q {
            best = take_best(best, SCORE_WORD_EXACT, "word-exact");
        } else if q.len() >= MIN_WORD_PREFIX_LEN && tok.starts_with(q) {
            best = take_best(best, SCORE_WORD_PREFIX, "word-prefix");
        }
    }

    // 2d) 有序多词：visual code / vs code（vs 可匹配连续词首字母）
    if tokens(q).len() >= 2 && ordered_token_match(q, name.as_ref()) {
        best = take_best(best, SCORE_TOKEN_SEQ, "token-seq");
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
        if let Some((score, _)) =
            fuzzy_name_or_tokens(q, &name).or_else(|| fuzzy_name_or_tokens(q, &display))
        {
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

/// Query 各词按序匹配名称词：整词、前缀，或连续若干词的首字母串（vs → visual studio）。
fn ordered_token_match(q: &str, name: &str) -> bool {
    let q_tokens = tokens(q);
    let n_tokens = tokens(name);
    if q_tokens.is_empty() || n_tokens.is_empty() {
        return false;
    }
    let mut ni = 0usize;
    for &qt in &q_tokens {
        let mut matched = false;
        while ni < n_tokens.len() {
            let nt = n_tokens[ni];
            if nt == qt || nt.starts_with(qt) {
                ni += 1;
                matched = true;
                break;
            }
            // 连续词首字母拼接（至少 2 词，长度与 qt 相同）
            if qt.chars().count() >= 2 {
                let mut acc = String::new();
                let mut j = ni;
                while j < n_tokens.len() && acc.chars().count() < qt.chars().count() {
                    if let Some(c) = n_tokens[j].chars().next() {
                        acc.push(c);
                    }
                    j += 1;
                }
                if acc == qt && j > ni + 1 {
                    ni = j;
                    matched = true;
                    break;
                }
            }
            ni += 1;
        }
        if !matched {
            return false;
        }
    }
    true
}

fn fuzzy_name_or_tokens(q: &str, name: &str) -> Option<(i32, usize)> {
    if let Some(hit) = fuzzy_match(q, name) {
        return Some(hit);
    }
    name.split_whitespace()
        .filter_map(|tok| fuzzy_match(q, tok))
        .max_by_key(|(score, _)| *score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_match_visual_code() {
        assert!(ordered_token_match("visual code", "visual studio code"));
    }

    #[test]
    fn ordered_match_vs_code_via_initials() {
        assert!(ordered_token_match("vs code", "visual studio code"));
    }

    #[test]
    fn ordered_match_rejects_wrong_order() {
        assert!(!ordered_token_match("zzz code", "visual studio code"));
        assert!(!ordered_token_match("code visual", "visual studio code"));
    }
}
