//! 候选验证编排：多通道证据合并为 MatchScore。
//!
//! 职责边界：
//! - `evidence`：证据类型与比较器
//! - `context`：每 Query 匹配上下文
//! - `align`：纯对齐/映射算法
//! - 本模块：通道编排（`verify_one`）与候选集验证

mod align;
mod context;
mod evidence;

pub use context::{CharBits, QueryContext};
pub use evidence::{
    cmp_ranked_hit, MatchEvidence, MatchField, MatchKind, RankedHit, ScoredHit, SCORE_NUCLEO_MAX,
    SCORE_SKIP,
};

use nucleo_matcher::Utf32Str;

use crate::model::SearchResult;
use crate::search::alias;
use crate::search::matcher::UserTarget;
use crate::search::ranker::{
    fuzzy_score, CONTEXT_COMPACT_DISCOUNT, CONTEXT_DISCOUNT, CONTEXT_PINYIN_DISCOUNT,
    SCORE_ACRONYM, SCORE_BUILTIN_ALIAS_EXACT, SCORE_COMPACT_EXACT, SCORE_COMPACT_SUBSTRING,
    SCORE_KEYWORD_PINYIN_EXACT, SCORE_KEYWORD_PINYIN_INNER, SCORE_KEYWORD_PINYIN_PREFIX,
    SCORE_NAME_EXACT, SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL, SCORE_PINYIN_INITIAL_INNER,
    SCORE_PREFIX, SCORE_SUBSTRING, SCORE_TOKEN_SEQ, SCORE_USER_ALIAS_EXACT, SCORE_WORD_EXACT,
    SCORE_WORD_PREFIX,
};
use crate::search::retrieval::channels::Candidates;
use crate::search::retrieval::doc::{DocId, IndexedDoc, RetrievalIndex};
use crate::search::retrieval::query::ParsedQuery;

use align::{
    fuzzy_name_or_tokens, loose_mixed_fragments, map_initial_index_to_display, map_nucleo_score,
    match_mixed_pinyin, ordered_mixed_align, ordered_skip_score, ordered_token_span,
};

const MIN_COMPACT_SUBSTR_LEN: usize = 3;
const MIN_WORD_PREFIX_LEN: usize = 2;
const MIN_ACRONYM_LEN: usize = 2;

/// 对候选集验证并评分。
pub fn verify_all(
    index: &RetrievalIndex,
    candidates: &Candidates,
    ctx: &mut QueryContext,
    user_targets: &[UserTarget],
) -> Vec<ScoredHit> {
    let mut hits = Vec::new();
    let mut candidate_ids: Vec<_> = candidates.ids.iter().copied().collect();
    candidate_ids.sort_unstable();
    for id in candidate_ids {
        let Some(doc) = index.doc(id) else { continue };
        if let Some(hit) = verify_one(doc, ctx, user_targets) {
            hits.push(ScoredHit {
                doc_id: id,
                score: hit.0,
                matched_by: hit.1.to_string(),
                evidence: hit.2,
            });
        }
    }
    hits
}

/// (score, matched_by, evidence)
type BestHit = (i32, &'static str, MatchEvidence);

fn take_best(best: Option<BestHit>, score: i32, kind: &'static str, evidence: MatchEvidence) -> Option<BestHit> {
    Some(match best {
        Some((bs, bk, be)) if bs > score => (bs, bk, be),
        // 同分用证据细排：连续/词边界/更早起点/更少跳空
        Some((bs, bk, be)) if bs == score => {
            if evidence.outranks(&be) {
                (score, kind, evidence)
            } else {
                (bs, bk, be)
            }
        }
        _ => (score, kind, evidence),
    })
}

fn verify_one(
    doc: &IndexedDoc,
    ctx: &mut QueryContext,
    user_targets: &[UserTarget],
) -> Option<BestHit> {
    let q = ctx.parsed;
    let name = doc.name.as_str();
    let display = doc.display.as_str();
    let q_raw = q.raw_norm.as_str();
    let q_compact = q.compact.as_str();
    let q_len = q.chars.len();

    let mut best: Option<BestHit> = None;

    // 0) 用户 Alias
    let hit_user = user_targets.iter().any(|t| match &t.id {
        Some(id) => doc.item.id == *id,
        None => {
            let n = t.name.to_lowercase();
            name == n.as_str() || name.contains(n.as_str()) || display.contains(n.as_str())
        }
    });
    if hit_user {
        best = take_best(
            best,
            SCORE_USER_ALIAS_EXACT,
            "user-alias-exact",
            MatchEvidence::exact(MatchField::Alias),
        );
    }

    // 1) 内置 Alias
    if let Some(fragments) = alias::targets_for(q_raw) {
        if fragments
            .iter()
            .any(|t| name.contains(t) || display.contains(t))
        {
            best = take_best(
                best,
                SCORE_BUILTIN_ALIAS_EXACT,
                "alias-exact",
                MatchEvidence::exact(MatchField::Alias),
            );
        }
    }

    // 2) Exact / Prefix / Substring
    for (field, candidate) in [(MatchField::Name, name), (MatchField::Display, display)] {
        if candidate == q_raw {
            best = take_best(
                best,
                SCORE_NAME_EXACT,
                "exact",
                MatchEvidence::exact(field),
            );
        } else if candidate.starts_with(q_raw) {
            best = take_best(
                best,
                SCORE_PREFIX,
                "prefix",
                MatchEvidence::prefix(field, q_len),
            );
        } else if let Some(start) = candidate.find(q_raw) {
            best = take_best(
                best,
                SCORE_SUBSTRING,
                "substring",
                MatchEvidence::contiguous(field, candidate[..start].chars().count(), q_len),
            );
        }
    }
    for candidate in &doc.keywords {
        if candidate == q_raw {
            best = take_best(
                best,
                SCORE_WORD_EXACT,
                "keyword-exact",
                MatchEvidence::exact(MatchField::Keyword),
            );
        } else if candidate.starts_with(q_raw) {
            best = take_best(
                best,
                SCORE_WORD_PREFIX,
                "keyword-prefix",
                MatchEvidence::prefix(MatchField::Keyword, q_len),
            );
        } else if let Some(start) = candidate.find(q_raw) {
            best = take_best(
                best,
                SCORE_SUBSTRING - 40,
                "keyword-substring",
                MatchEvidence::contiguous(
                    MatchField::Keyword,
                    candidate[..start].chars().count(),
                    q_len,
                ),
            );
        }
    }

    // 2b) Compact
    if q_compact.len() >= 2 {
        for candidate in [doc.compact_name.as_str(), doc.compact_display.as_str()] {
            if candidate == q_compact {
                best = take_best(
                    best,
                    SCORE_COMPACT_EXACT,
                    "compact-exact",
                    MatchEvidence::exact(MatchField::Name),
                );
            } else if q_compact.len() >= MIN_COMPACT_SUBSTR_LEN {
                if let Some(start) = candidate.find(q_compact) {
                    best = take_best(
                        best,
                        SCORE_COMPACT_SUBSTRING,
                        "compact-substring",
                        MatchEvidence::contiguous(
                            MatchField::Name,
                            candidate[..start].chars().count(),
                            q_compact.chars().count(),
                        ),
                    );
                }
            }
        }
        for candidate in &doc.keyword_compacts {
            if candidate == q_compact {
                best = take_best(
                    best,
                    SCORE_WORD_EXACT,
                    "keyword-compact-exact",
                    MatchEvidence::exact(MatchField::Keyword),
                );
            } else if q_compact.len() >= MIN_COMPACT_SUBSTR_LEN {
                if let Some(start) = candidate.find(q_compact) {
                    best = take_best(
                        best,
                        SCORE_COMPACT_SUBSTRING - 60,
                        "keyword-compact-substring",
                        MatchEvidence::contiguous(
                            MatchField::Keyword,
                            candidate[..start].chars().count(),
                            q_compact.chars().count(),
                        ),
                    );
                }
            }
        }
    }

    // 2c) 词边界
    for tok in &doc.tokens {
        if tok == q_raw {
            best = take_best(
                best,
                SCORE_WORD_EXACT,
                "word-exact",
                MatchEvidence::exact(MatchField::Name),
            );
        } else if q_raw.chars().count() >= MIN_WORD_PREFIX_LEN && tok.starts_with(q_raw) {
            best = take_best(
                best,
                SCORE_WORD_PREFIX,
                "word-prefix",
                MatchEvidence::prefix(MatchField::Name, q_len),
            );
        }
    }

    // 2d) 有序多词
    if q.tokens.len() >= 2 {
        if let Some((start, gaps)) = ordered_token_span(&q.tokens, &doc.tokens, name) {
            best = take_best(
                best,
                SCORE_TOKEN_SEQ,
                "token-seq",
                MatchEvidence {
                    field: MatchField::Name,
                    kind: MatchKind::Contiguous,
                    start,
                    span: q_len,
                    gaps,
                    edit_cost: 0,
                },
            );
        }
    }

    // 2e) 英文缩写
    if q_raw.chars().count() >= MIN_ACRONYM_LEN && !doc.acronym.is_empty() && doc.acronym == q_raw {
        best = take_best(
            best,
            SCORE_ACRONYM,
            "acronym",
            MatchEvidence {
                field: MatchField::Name,
                kind: MatchKind::Acronym,
                start: 0,
                span: q_len,
                gaps: 0,
                edit_cost: 0,
            },
        );
    }

    // 2f) 混输：有序对齐给高分；仅片段各自碰巧出现作宽召回低分
    if q.mixed && !q.ordered_mixed_parts.is_empty() {
        for (field, src) in [(MatchField::Name, name), (MatchField::Display, display)] {
            if let Some((start, gaps, span)) = ordered_mixed_align(
                &q.ordered_mixed_parts,
                src,
                &doc.pinyin,
                &doc.pinyin_syllables,
            ) {
                best = take_best(
                    best,
                    SCORE_WORD_EXACT,
                    "mixed-cjk-latin-ordered",
                    MatchEvidence {
                        field,
                        kind: MatchKind::Contiguous,
                        start,
                        span,
                        gaps,
                        edit_cost: 0,
                    },
                );
            } else if loose_mixed_fragments(q, src, doc) {
                // 宽召回：片段分别出现但未按名称/拼音顺序对齐，不得与有序同权。
                let start = q
                    .cjk_parts
                    .first()
                    .and_then(|p| src.find(p.as_str()))
                    .map(|b| src[..b].chars().count())
                    .unwrap_or(usize::MAX);
                best = take_best(
                    best,
                    SCORE_WORD_PREFIX - 40,
                    "mixed-cjk-latin-loose",
                    MatchEvidence {
                        field,
                        kind: MatchKind::Contiguous,
                        start,
                        span: q.cjk_parts.iter().map(|p| p.chars().count()).sum::<usize>().max(1),
                        gaps: usize::MAX / 4,
                        edit_cost: 0,
                    },
                );
            }
        }
    }

    // 3) 拼音（全拼 / 首字母 / ib-pinyin 混合+多音字）
    // 位置映射回原名称：全拼/首字母与 display 字符序对齐时用音节起点，否则记 MAX。
    if !doc.pinyin.is_empty() {
        if doc.pinyin == q_raw {
            best = take_best(
                best,
                SCORE_PINYIN_EXACT,
                "pinyin-exact",
                MatchEvidence {
                    field: MatchField::Pinyin,
                    kind: MatchKind::Exact,
                    start: 0,
                    span: doc.item.display_name.chars().count(),
                    gaps: 0,
                    edit_cost: 0,
                },
            );
        } else if doc.pinyin.starts_with(q_raw) {
            best = take_best(
                best,
                SCORE_PINYIN_EXACT - 50,
                "pinyin-prefix",
                MatchEvidence {
                    field: MatchField::Pinyin,
                    kind: MatchKind::Prefix,
                    start: 0,
                    span: q_len,
                    gaps: 0,
                    edit_cost: 0,
                },
            );
        }
    }
    if !doc.pinyin_initials.is_empty() {
        if doc.pinyin_initials == q_raw {
            best = take_best(
                best,
                SCORE_PINYIN_INITIAL,
                "pinyin-initial",
                MatchEvidence {
                    field: MatchField::Pinyin,
                    kind: MatchKind::Exact,
                    start: 0,
                    span: doc.item.display_name.chars().count(),
                    gaps: 0,
                    edit_cost: 0,
                },
            );
        } else if doc.pinyin_initials.starts_with(q_raw) {
            // 单字母也允许前缀（k → 控制面板 kzmb）；分略低于多字母，避免压过名称前缀
            let score = if q_raw.chars().count() == 1 {
                SCORE_PINYIN_INITIAL - 60
            } else {
                SCORE_PINYIN_INITIAL - 40
            };
            best = take_best(
                best,
                score,
                "pinyin-initial-prefix",
                MatchEvidence {
                    field: MatchField::Pinyin,
                    kind: MatchKind::Prefix,
                    start: 0,
                    span: q_len,
                    gaps: 0,
                    edit_cost: 0,
                },
            );
        } else if let Some(byte_at) = doc.pinyin_initials.find(q_raw) {
            // 映射回原名称：首字母串第 i 位对应 display 第 i 个非空白字符
            let start = map_initial_index_to_display(doc, byte_at);
            let score = if q_raw.chars().count() == 1 {
                SCORE_PINYIN_INITIAL_INNER + 40
            } else {
                SCORE_PINYIN_INITIAL_INNER
            };
            best = take_best(
                best,
                score,
                "pinyin-initial-inner",
                MatchEvidence {
                    field: MatchField::Pinyin,
                    kind: MatchKind::Contiguous,
                    start,
                    span: q_len,
                    gaps: 0,
                    edit_cost: 0,
                },
            );
        }
    }
    // ib-pinyin：混合全拼/简拼/多音字（对原文匹配，覆盖非默认读音）；允许混输
    if q.has_ascii_alnum {
        if let Some(matcher) = ctx.pinyin.as_ref() {
            let haystack = &doc.item.display_name;
            if let Some(m) = matcher.find(haystack.as_str()) {
                // 前缀式匹配（is_pattern_partial）略低于完整全拼；位置映射回原名称字符下标
                let start = haystack[..m.start()].chars().count();
                let end = haystack[..m.end()].chars().count();
                let span = end.saturating_sub(start).max(q_len);
                best = take_best(
                    best,
                    SCORE_PINYIN_EXACT - 20,
                    "pinyin-ib",
                    MatchEvidence {
                        field: MatchField::Pinyin,
                        kind: MatchKind::Contiguous,
                        start,
                        span,
                        gaps: 0,
                        edit_cost: 0,
                    },
                );
            }
        } else if !doc.pinyin_syllables.is_empty() && q_raw.chars().count() >= 2 {
            if let Some((score, kind, start)) = match_mixed_pinyin(&doc.pinyin_syllables, q_raw, name)
            {
                best = take_best(
                    best,
                    score,
                    kind,
                    MatchEvidence {
                        field: MatchField::Pinyin,
                        kind: MatchKind::Contiguous,
                        start,
                        span: q_len,
                        gaps: 0,
                        edit_cost: 0,
                    },
                );
            }
        }
    }

    // 可信搜索词的全拼/简拼保留独立证据，避免被当作普通 token 前缀低估。
    if q_raw.chars().count() >= 2 {
        for full in &doc.keyword_full_pinyin {
            if full == q_raw {
                best = take_best(
                    best,
                    SCORE_KEYWORD_PINYIN_EXACT,
                    "keyword-pinyin-exact",
                    MatchEvidence::exact(MatchField::Keyword),
                );
            } else if full.starts_with(q_raw) {
                best = take_best(
                    best,
                    SCORE_KEYWORD_PINYIN_PREFIX,
                    "keyword-pinyin-prefix",
                    MatchEvidence::prefix(MatchField::Keyword, q_len),
                );
            }
        }
        for initials in &doc.keyword_initials {
            if initials == q_raw {
                best = take_best(
                    best,
                    SCORE_KEYWORD_PINYIN_EXACT,
                    "keyword-initials-exact",
                    MatchEvidence::exact(MatchField::Keyword),
                );
            } else if initials.starts_with(q_raw) {
                best = take_best(
                    best,
                    SCORE_KEYWORD_PINYIN_PREFIX,
                    "keyword-initials-prefix",
                    MatchEvidence::prefix(MatchField::Keyword, q_len),
                );
            } else if let Some(start) = initials.find(q_raw) {
                best = take_best(
                    best,
                    SCORE_KEYWORD_PINYIN_INNER,
                    "keyword-initials-inner",
                    MatchEvidence::contiguous(MatchField::Keyword, start, q_len),
                );
            }
        }
    }

    // 3b) Windows 页面级标准搜索资源：只提供候选证据，不压过真实名称或可信别名。
    if q_raw.chars().count() >= 2 {
        for field in &doc.context_fields {
            if field == q_raw {
                best = take_best(
                    best,
                    SCORE_WORD_EXACT - CONTEXT_DISCOUNT,
                    "context-exact",
                    MatchEvidence::exact(MatchField::Context),
                );
            } else if field.starts_with(q_raw) {
                best = take_best(
                    best,
                    SCORE_WORD_PREFIX - CONTEXT_DISCOUNT,
                    "context-prefix",
                    MatchEvidence::prefix(MatchField::Context, q_len),
                );
            } else if let Some(start) = field.find(q_raw) {
                best = take_best(
                    best,
                    SCORE_SUBSTRING - CONTEXT_DISCOUNT,
                    "context-substring",
                    MatchEvidence::contiguous(MatchField::Context, start, q_len),
                );
            }
        }
        for term in &doc.context_terms {
            if term == q_raw {
                best = take_best(
                    best,
                    SCORE_WORD_EXACT - CONTEXT_DISCOUNT,
                    "context-word-exact",
                    MatchEvidence::exact(MatchField::Context),
                );
            } else if term.starts_with(q_raw) {
                best = take_best(
                    best,
                    SCORE_WORD_PREFIX - CONTEXT_DISCOUNT,
                    "context-word-prefix",
                    MatchEvidence::prefix(MatchField::Context, q_len),
                );
            }
        }
        if q_compact.len() >= 2 {
            for compact in &doc.context_compacts {
                if compact == q_compact {
                    best = take_best(
                        best,
                        SCORE_COMPACT_EXACT - CONTEXT_COMPACT_DISCOUNT,
                        "context-compact-exact",
                        MatchEvidence::exact(MatchField::Context),
                    );
                } else if q_compact.len() >= MIN_COMPACT_SUBSTR_LEN {
                    if let Some(start) = compact.find(q_compact) {
                        best = take_best(
                            best,
                            SCORE_COMPACT_SUBSTRING - CONTEXT_COMPACT_DISCOUNT,
                            "context-compact-substring",
                            MatchEvidence::contiguous(MatchField::Context, start, q_len),
                        );
                    }
                }
            }
        }
        for full in &doc.context_full_pinyin {
            if full == q_raw {
                best = take_best(
                    best,
                    SCORE_PINYIN_EXACT - CONTEXT_PINYIN_DISCOUNT,
                    "context-pinyin-exact",
                    MatchEvidence::exact(MatchField::Context),
                );
            } else if full.starts_with(q_raw) {
                best = take_best(
                    best,
                    SCORE_PINYIN_EXACT - CONTEXT_PINYIN_DISCOUNT - 40,
                    "context-pinyin-prefix",
                    MatchEvidence::prefix(MatchField::Context, q_len),
                );
            }
        }
        for initials in &doc.context_initials {
            if initials == q_raw {
                best = take_best(
                    best,
                    SCORE_PINYIN_INITIAL - CONTEXT_PINYIN_DISCOUNT,
                    "context-initials-exact",
                    MatchEvidence::exact(MatchField::Context),
                );
            } else if initials.starts_with(q_raw) {
                best = take_best(
                    best,
                    SCORE_PINYIN_INITIAL - CONTEXT_PINYIN_DISCOUNT - 40,
                    "context-initials-prefix",
                    MatchEvidence::prefix(MatchField::Context, q_len),
                );
            } else if let Some(start) = initials.find(q_raw) {
                best = take_best(
                    best,
                    SCORE_PINYIN_INITIAL_INNER - CONTEXT_PINYIN_DISCOUNT,
                    "context-initials-inner",
                    MatchEvidence::contiguous(MatchField::Context, start, q_len),
                );
            }
        }
    }

    // 4) 有序跳字（省略中间字符）；短 Query 选择性差，不走
    if best.map(|(s, _, _)| s < SCORE_SKIP).unwrap_or(true) && q_raw.chars().count() >= 4 {
        if let Some((score, start, gaps)) = ordered_skip_score(q_raw, name) {
            best = take_best(
                best,
                score,
                "skip",
                MatchEvidence {
                    field: MatchField::Name,
                    kind: MatchKind::Skip,
                    start,
                    span: q_len + gaps,
                    gaps,
                    edit_cost: 0,
                },
            );
        } else if let Some((score, start, gaps)) = ordered_skip_score(q_raw, display) {
            best = take_best(
                best,
                score,
                "skip",
                MatchEvidence {
                    field: MatchField::Display,
                    kind: MatchKind::Skip,
                    start,
                    span: q_len + gaps,
                    gaps,
                    edit_cost: 0,
                },
            );
        } else {
            for (keyword, bits) in doc.keywords.iter().zip(&doc.keyword_bits) {
                if bits.contains_all(&q.chars) {
                    if let Some((score, start, gaps)) = ordered_skip_score(q_raw, keyword) {
                        best = take_best(
                            best,
                            score - 30,
                            "keyword-skip",
                            MatchEvidence {
                                field: MatchField::Keyword,
                                kind: MatchKind::Skip,
                                start,
                                span: q_len + gaps,
                                gaps,
                                edit_cost: 0,
                            },
                        );
                        break;
                    }
                }
            }
        }
    }

    // 5) nucleo-matcher 非连续对齐：只补充证据，不否决已有命中
    if best.map(|(s, _, _)| s < SCORE_NUCLEO_MAX).unwrap_or(true) {
        if ctx.nucleo_atom.is_some() {
            let hay = Utf32Str::new(name, &mut ctx.hay_buf);
            let atom = ctx.nucleo_atom.as_ref().unwrap();
            if let Some(raw) = atom.score(hay, &mut ctx.nucleo) {
                let mapped = map_nucleo_score(raw);
                if mapped > 0 {
                    best = take_best(
                        best,
                        mapped,
                        "nucleo",
                        MatchEvidence {
                            field: MatchField::Name,
                            kind: MatchKind::Skip,
                            start: usize::MAX,
                            span: q_len,
                            gaps: 1,
                            edit_cost: 0,
                        },
                    );
                }
            }
        }
    }

    // 6) Fuzzy / SymSpell 验证（编辑距离）；1 字符不纠错。
    // skip 等弱证据不得挡住更高分的编辑距离命中（如 crome→chrome）。
    // 召回可来自拼音等派生字段的删除变体；验证必须对对应字段算真实距离。
    if best.map(|(s, _, _)| s < fuzzy_score(1)).unwrap_or(true) && q_raw.chars().count() >= 2 {
        if let Some((score, dist)) =
            fuzzy_name_or_tokens(q_raw, name).or_else(|| fuzzy_name_or_tokens(q_raw, display))
        {
            best = take_best(
                best,
                score,
                "fuzzy",
                MatchEvidence {
                    field: MatchField::Name,
                    kind: MatchKind::Fuzzy,
                    start: usize::MAX,
                    span: q_len.saturating_sub(dist),
                    gaps: 0,
                    edit_cost: dist,
                },
            );
        } else if let Some((score, dist)) = doc
            .keywords
            .iter()
            .filter_map(|keyword| fuzzy_name_or_tokens(q_raw, keyword))
            .max_by_key(|(score, _)| *score)
        {
            best = take_best(
                best,
                score - 30,
                "keyword-fuzzy",
                MatchEvidence {
                    field: MatchField::Keyword,
                    kind: MatchKind::Fuzzy,
                    start: usize::MAX,
                    span: q_len.saturating_sub(dist),
                    gaps: 0,
                    edit_cost: dist,
                },
            );
        } else if let Some((score, dist)) = fuzzy_derived_pinyin(doc, q_raw) {
            best = take_best(
                best,
                score - 20,
                "pinyin-fuzzy",
                MatchEvidence {
                    field: MatchField::Pinyin,
                    kind: MatchKind::Fuzzy,
                    start: usize::MAX,
                    span: q_len.saturating_sub(dist),
                    gaps: 0,
                    edit_cost: dist,
                },
            );
        }
    }

    best
}

/// 对拼音/简拼/关键词全拼做受限真实距离验证（字段级纠错，不新开引擎）。
fn fuzzy_derived_pinyin(doc: &IndexedDoc, q_raw: &str) -> Option<(i32, usize)> {
    use crate::search::fuzzy::fuzzy_match;
    let mut best: Option<(i32, usize)> = None;
    let mut consider = |hit: Option<(i32, usize)>| {
        if let Some(hit) = hit {
            if best.map(|(s, _)| hit.0 > s).unwrap_or(true) {
                best = Some(hit);
            }
        }
    };
    consider(fuzzy_match(q_raw, &doc.pinyin));
    consider(fuzzy_match(q_raw, &doc.pinyin_initials));
    for full in &doc.keyword_full_pinyin {
        consider(fuzzy_match(q_raw, full));
    }
    for initials in &doc.keyword_initials {
        consider(fuzzy_match(q_raw, initials));
    }
    best
}


/// 参考排序：无剪枝全量验证后排序（测试缝）。
pub fn reference_search(
    docs: &[IndexedDoc],
    q: &ParsedQuery,
    user_targets: &[UserTarget],
    max_results: usize,
) -> Vec<SearchResult> {
    // 参考路径也用同一验证语义；ib-pinyin 需要 Query 生命周期，这里用临时空索引构建上下文
    let empty = RetrievalIndex::build(&[], &[]);
    let mut ctx = QueryContext::build(q, &empty);
    let mut hits: Vec<(i32, usize, String, DocId, String)> = Vec::new();
    for doc in docs {
        if let Some((score, matched_by, _evidence)) = verify_one(doc, &mut ctx, user_targets) {
            hits.push((
                score,
                doc.item.name.chars().count(),
                doc.item.name.to_lowercase(),
                doc.id,
                matched_by.to_string(),
            ));
        }
    }
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    hits.truncate(max_results);
    hits.into_iter()
        .map(|(score, _, _, id, matched_by)| {
            let doc = docs.iter().find(|d| d.id == id).unwrap();
            SearchResult::scored(doc.item.clone(), score, matched_by)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::align::*;
    use super::*;
    use crate::search::retrieval::query::MixedPart;
    use ib_pinyin::{matcher::PinyinMatcher, pinyin::PinyinNotation};

    #[test]
    fn skip_matches_google_chrome() {
        // googchrome 跳过 google 中的 l、e 与空格，按序命中
        let hit = ordered_skip_score("googchrome", "google chrome");
        assert!(hit.is_some(), "googchrome 应有序命中 google chrome");
        let (score, start, _gaps) = hit.unwrap();
        assert!(score <= SCORE_SKIP);
        assert_eq!(start, 0);
    }

    #[test]
    fn skip_rejects_wrong_order() {
        assert!(ordered_skip_score("emorhclod", "google chrome").is_none());
    }

    #[test]
    fn evidence_prefers_earlier_and_tighter_match() {
        let early = MatchEvidence::contiguous(MatchField::Name, 0, 3);
        let late = MatchEvidence::contiguous(MatchField::Name, 5, 3);
        assert!(early.outranks(&late));
        let tight = MatchEvidence::contiguous(MatchField::Name, 0, 4);
        let loose = MatchEvidence::contiguous(MatchField::Name, 0, 2);
        assert!(tight.outranks(&loose));
        let skip_close = MatchEvidence {
            field: MatchField::Name,
            kind: MatchKind::Skip,
            start: 0,
            span: 4,
            gaps: 1,
            edit_cost: 0,
        };
        let skip_far = MatchEvidence {
            field: MatchField::Name,
            kind: MatchKind::Skip,
            start: 0,
            span: 4,
            gaps: 3,
            edit_cost: 0,
        };
        assert!(skip_close.outranks(&skip_far));
        let cheap = MatchEvidence {
            field: MatchField::Name,
            kind: MatchKind::Fuzzy,
            start: usize::MAX,
            span: 3,
            gaps: 0,
            edit_cost: 1,
        };
        let dear = MatchEvidence {
            field: MatchField::Name,
            kind: MatchKind::Fuzzy,
            start: usize::MAX,
            span: 3,
            gaps: 0,
            edit_cost: 2,
        };
        assert!(cheap.outranks(&dear));
    }

    #[test]
    fn mixed_pinyin_wxin() {
        let syl = vec!["wei".to_string(), "xin".to_string()];
        let hit = match_mixed_pinyin(&syl, "wxin", "微信");
        assert!(hit.is_some(), "wxin 应命中 微信 音节");
        let hit = match_mixed_pinyin(&syl, "weixin", "微信");
        assert!(hit.is_some());
        let hit = match_mixed_pinyin(&syl, "wx", "微信");
        assert!(hit.is_some());
        assert!(match_mixed_pinyin(&syl, "zzz", "微信").is_none());
    }

    #[test]
    fn ib_pinyin_mixed_and_polyphone() {
        // 官方语义：混合全拼/简拼 + 多音字数据
        let m = PinyinMatcher::builder("pysousuoeve")
            .pinyin_notations(PinyinNotation::Ascii | PinyinNotation::AsciiFirstLetter)
            .build();
        assert!(m.is_match("拼音搜索Everything"), "ib-pinyin 混合命中");

        let m = PinyinMatcher::builder("wxin")
            .pinyin_notations(PinyinNotation::Ascii | PinyinNotation::AsciiFirstLetter)
            .is_pattern_partial(true)
            .build();
        assert!(m.is_match("微信"), "wxin 应命中微信");
    }

    #[test]
    fn ordered_mixed_accepts_hanzi_then_remaining_pinyin() {
        let parts = vec![MixedPart::Cjk("微".into()), MixedPart::Latin("xin".into())];
        let syl = vec!["wei".to_string(), "xin".to_string()];
        let hit = ordered_mixed_align(&parts, "微信", "weixin", &syl);
        assert!(hit.is_some(), "微xin 应有序对齐到微信");
    }

    #[test]
    fn ordered_mixed_rejects_pinyin_restatement_of_full_hanzi() {
        let parts = vec![
            MixedPart::Cjk("微信".into()),
            MixedPart::Latin("xin".into()),
        ];
        let syl = vec!["wei".to_string(), "xin".to_string()];
        let hit = ordered_mixed_align(&parts, "微信", "weixin", &syl);
        assert!(hit.is_none(), "微信xin 不得视为有序对齐（复用已消费拼音）");
    }

    #[test]
    fn ordered_mixed_rejects_pinyin_before_hanzi() {
        let parts = vec![
            MixedPart::Latin("xin".into()),
            MixedPart::Cjk("微信".into()),
        ];
        let syl = vec!["wei".to_string(), "xin".to_string()];
        let hit = ordered_mixed_align(&parts, "微信", "weixin", &syl);
        assert!(hit.is_none(), "xin微信 不得视为有序对齐");
    }

    #[test]
    fn char_bits_contains_all() {
        let bits = CharBits::from_str("google chrome");
        assert!(bits.contains_all(&['g', 'o', 'c']));
        assert!(!bits.contains_all(&['z']));
    }

    #[test]
    fn word_acronym_and_ordered_tokens() {
        use crate::search::normalizer::tokens;
        let ac: String = tokens("neat download manager")
            .iter()
            .filter_map(|t| t.chars().next())
            .collect();
        assert_eq!(ac, "ndm");
        let q: Vec<String> = tokens("visual code")
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let n: Vec<String> = tokens("visual studio code")
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        assert!(ordered_token_span(&q, &n, "visual studio code").is_some());
        let wrong: Vec<String> = tokens("code visual")
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        assert!(ordered_token_span(&wrong, &n, "visual studio code").is_none());
    }
}

