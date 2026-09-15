//! 纯文本对齐与位置映射 helper。
//!
//! 职责：有序多词/跳字/混输/音节匹配的几何计算，不依赖 RetrievalIndex 热路径状态。

use crate::search::fuzzy::fuzzy_match;
use crate::search::ranker::{
    fuzzy_score, SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL,
};
use crate::search::retrieval::doc::IndexedDoc;
use crate::search::retrieval::query::{MixedPart, ParsedQuery};

use super::{SCORE_NUCLEO_MAX, SCORE_SKIP};

/// 跳字最大额外跨度（Query 长度之外允许跳过的字符数）。
const SKIP_EXTRA_SPAN: usize = 3;

/// 拼音首字母串中的字节偏移 → display **原始**字符下标（含空格）。
/// 每个非空白字符对应一位首字母；返回值须与 name/token 的 start 同一坐标系。
pub(crate) fn map_initial_index_to_display(doc: &IndexedDoc, initial_byte_at: usize) -> usize {
    let initials = doc.pinyin_initials.as_str();
    if initial_byte_at > initials.len() {
        return usize::MAX;
    }
    let idx = initials[..initial_byte_at].chars().count();
    let mut non_ws = 0usize;
    for (i, ch) in doc.item.display_name.chars().enumerate() {
        if ch.is_whitespace() {
            continue;
        }
        if non_ws == idx {
            return i;
        }
        non_ws += 1;
    }
    usize::MAX
}

/// nucleo u16 分映射到本内核分数域（低质量，不抢精确/前缀）。
pub(crate) fn map_nucleo_score(raw: u16) -> i32 {
    // nucleo 分数量级约 0..~200+；线性压到 100..SCORE_NUCLEO_MAX
    let s = (raw as i32).clamp(0, 200);
    100 + (s * (SCORE_NUCLEO_MAX - 100)) / 200
}

pub(crate) fn fuzzy_name_or_tokens(q: &str, name: &str) -> Option<(i32, usize)> {
    if let Some(hit) = fuzzy_match(q, name) {
        return Some(hit);
    }
    name.split_whitespace()
        .filter_map(|tok| fuzzy_match(q, tok))
        .max_by_key(|(score, _)| *score)
}

/// Query 各词按序匹配名称词：整词、前缀，或连续词首字母串。
/// 返回 (首个命中词在原名称中的字符起点, 跳过的名称词元数)。
pub(crate) fn ordered_token_span(
    q_tokens: &[String],
    n_tokens: &[String],
    name: &str,
) -> Option<(usize, usize)> {
    if q_tokens.is_empty() || n_tokens.is_empty() {
        return None;
    }
    // 名称词在规范化名称中的字符起点（按空白切分）
    let mut token_starts: Vec<usize> = Vec::with_capacity(n_tokens.len());
    {
        let mut char_i = 0usize;
        let mut in_tok = false;
        for ch in name.chars() {
            if ch.is_whitespace() {
                in_tok = false;
            } else if !in_tok {
                token_starts.push(char_i);
                in_tok = true;
            }
            char_i += 1;
        }
    }
    let start_of = |ni: usize| -> usize { token_starts.get(ni).copied().unwrap_or(usize::MAX) };

    let mut ni = 0usize;
    let mut gaps = 0usize;
    let mut first_start = None;
    for qt in q_tokens {
        let mut matched = false;
        while ni < n_tokens.len() {
            let nt = n_tokens[ni].as_str();
            if nt == qt || nt.starts_with(qt.as_str()) {
                if first_start.is_none() {
                    first_start = Some(start_of(ni));
                }
                ni += 1;
                matched = true;
                break;
            }
            if qt.chars().count() >= 2 {
                let mut acc = String::new();
                let mut j = ni;
                while j < n_tokens.len() && acc.chars().count() < qt.chars().count() {
                    if let Some(c) = n_tokens[j].chars().next() {
                        acc.push(c);
                    }
                    j += 1;
                }
                if acc == *qt && j > ni + 1 {
                    if first_start.is_none() {
                        first_start = Some(start_of(ni));
                    }
                    ni = j;
                    matched = true;
                    break;
                }
            }
            ni += 1;
            gaps += 1;
        }
        if !matched {
            return None;
        }
    }
    Some((first_start?, gaps))
}

/// 按 Query 混输段顺序对齐名称 + 拼音流。
/// 成功返回 (名称字符起点, 跳空, 名称覆盖跨度)。
pub(crate) fn ordered_mixed_align(
    parts: &[MixedPart],
    name: &str,
    pinyin: &str,
    syllables: &[String],
) -> Option<(usize, usize, usize)> {
    if parts.is_empty() || name.is_empty() {
        return None;
    }
    let name_chars: Vec<char> = name.chars().collect();
    let syllables_for_name = syllables.len() == name_chars.len();
    let mut name_i = 0usize;
    let mut py_i = 0usize;
    let mut first_start = None;
    let mut end = 0usize;
    let mut gaps = 0usize;

    let name_index_at_pinyin_end = |abs_end: usize| -> usize {
        if !syllables_for_name {
            return name_chars.len();
        }
        let mut acc = 0usize;
        let mut i = 0usize;
        for syl in syllables {
            if acc >= abs_end {
                break;
            }
            acc += syl.len();
            i += 1;
        }
        i.min(name_chars.len())
    };

    for part in parts {
        match part {
            MixedPart::Cjk(s) => {
                let need: Vec<char> = s.chars().collect();
                if need.is_empty() {
                    continue;
                }
                let mut found = None;
                let mut search = name_i;
                while search + need.len() <= name_chars.len() {
                    if name_chars[search..search + need.len()] == need[..] {
                        found = Some(search);
                        break;
                    }
                    search += 1;
                }
                let start = found?;
                if start > name_i {
                    gaps += 1;
                }
                if first_start.is_none() {
                    first_start = Some(start);
                }
                name_i = start + need.len();
                end = end.max(name_i);
                if syllables_for_name {
                    py_i = syllables[..name_i].iter().map(|s| s.len()).sum();
                }
            }
            MixedPart::Latin(s) => {
                if s.is_empty() {
                    continue;
                }
                let need: Vec<char> = s.chars().collect();
                let mut found_name = None;
                let mut search = name_i;
                while search + need.len() <= name_chars.len() {
                    if name_chars[search..search + need.len()] == need[..] {
                        found_name = Some(search);
                        break;
                    }
                    search += 1;
                }
                if let Some(start) = found_name {
                    if start > name_i {
                        gaps += 1;
                    }
                    if first_start.is_none() {
                        first_start = Some(start);
                    }
                    name_i = start + need.len();
                    end = end.max(name_i);
                    if syllables_for_name {
                        py_i = syllables[..name_i].iter().map(|x| x.len()).sum();
                    }
                    continue;
                }
                // 拼音流从当前游标继续，并推进名称下标，保证不倒序复用前面音节
                if py_i < pinyin.len() {
                    if let Some(rel) = pinyin[py_i..].find(s.as_str()) {
                        let abs = py_i + rel;
                        py_i = abs + s.len();
                        name_i = name_i.max(name_index_at_pinyin_end(py_i));
                        end = end.max(name_i);
                        if first_start.is_none() {
                            first_start = Some(usize::MAX);
                        }
                        continue;
                    }
                }
                // 拒绝复用刚匹配汉字区域的拼音（微信xin）；这类只能走宽召回。
                return None;
            }
        }
    }

    let start = first_start?;
    let span = if start == usize::MAX {
        name_chars.len().max(1)
    } else {
        end.saturating_sub(start).max(1)
    };
    Some((start, gaps, span))
}

/// 宽召回：汉字段与拉丁段分别能命中，但不要求顺序。
pub(crate) fn loose_mixed_fragments(q: &ParsedQuery, src: &str, doc: &IndexedDoc) -> bool {
    if q.cjk_parts.is_empty() {
        return false;
    }
    let cjk_hit = q
        .cjk_parts
        .iter()
        .all(|part| src.contains(part.as_str()));
    if !cjk_hit {
        return false;
    }
    q.latin_parts.iter().all(|part| {
        (!doc.pinyin.is_empty() && doc.pinyin.contains(part.as_str()))
            || (!doc.pinyin_initials.is_empty() && doc.pinyin_initials.contains(part.as_str()))
            || src.contains(part.as_str())
            || doc
                .tokens
                .iter()
                .any(|t| t.starts_with(part.as_str()) || t.contains(part.as_str()))
    })
}

/// 有序子序列命中：字符按序出现，跨度与词边界受控。
/// 返回 (score, 原文本字符起点, extra_gaps)。
pub(crate) fn ordered_skip_score(query: &str, text: &str) -> Option<(i32, usize, usize)> {
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = text.chars().collect();
    if q.is_empty() || t.is_empty() || q.len() < 2 || q.len() > t.len() {
        return None;
    }
    let max_span = q.len() + SKIP_EXTRA_SPAN;
    // 对每个起点找最早终点，取最小跨度窗口；同跨度取更早起点
    let mut best_span = usize::MAX;
    let mut best_start = usize::MAX;
    for start in 0..t.len() {
        let mut qi = 0usize;
        for ti in start..t.len() {
            if t[ti] == q[qi] {
                qi += 1;
                if qi == q.len() {
                    let span = ti - start + 1;
                    if span < best_span || (span == best_span && start < best_start) {
                        best_span = span;
                        best_start = start;
                    }
                    break;
                }
            }
        }
        if best_span == q.len() {
            break; // 已是最短可能
        }
    }
    if best_span == usize::MAX {
        return None;
    }
    if best_span > max_span {
        return None;
    }
    // 连续时不应走 skip（substring 更高）；这里只在非连续时给分
    if best_span == q.len() {
        return None;
    }
    // 跨度越小分越高
    let gap = best_span.saturating_sub(q.len());
    let score = SCORE_SKIP - (gap as i32) * 8;
    Some((score.clamp(fuzzy_score(2), SCORE_SKIP), best_start, gap))
}

/// 混合全拼/简拼：音节序列上匹配 query。
/// 例：["wei","xin"] 可命中 weixin / wx / wxin / weix。
/// 返回 (score, kind, 首个命中音节在原名称中的字符起点)。
pub(crate) fn match_mixed_pinyin(
    syllables: &[String],
    query: &str,
    name: &str,
) -> Option<(i32, &'static str, usize)> {
    if syllables.is_empty() || query.is_empty() {
        return None;
    }
    // 音节与名称字符一一对应（汉字一音一字，拉丁连续段一音多字时按前缀对齐）
    let name_chars: Vec<char> = name.chars().collect();
    let syllables_for_name = syllables.len() == name_chars.len();
    let mut rest = query;
    let mut used_full = 0usize;
    let mut used_init = 0usize;
    let mut first_syl = None;
    for (si, syl) in syllables.iter().enumerate() {
        if rest.is_empty() {
            break;
        }
        if let Some(rem) = rest.strip_prefix(syl.as_str()) {
            if first_syl.is_none() && rem.len() < query.len() {
                first_syl = Some(si);
            }
            used_full += 1;
            rest = rem;
            continue;
        }
        if let Some(first) = syl.chars().next() {
            if rest.starts_with(first) {
                if first_syl.is_none() {
                    first_syl = Some(si);
                }
                used_init += 1;
                rest = &rest[first.len_utf8()..];
                continue;
            }
        }
        return None;
    }
    if !rest.is_empty() {
        return None;
    }
    let start = match first_syl {
        Some(si) if syllables_for_name => si.min(name_chars.len().saturating_sub(1)),
        // 无法一一映射时记未知，不当作最优起点
        Some(_) => usize::MAX,
        None => usize::MAX,
    };
    if used_init == 0 && used_full == syllables.len() {
        Some((SCORE_PINYIN_EXACT, "pinyin-mixed-exact", start))
    } else if used_full > 0 {
        Some((SCORE_PINYIN_EXACT - 30, "pinyin-mixed", start))
    } else {
        Some((SCORE_PINYIN_INITIAL, "pinyin-mixed-initial", start))
    }
}


