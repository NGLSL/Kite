//! 候选验证：多通道证据合并为 MatchScore。
//! 精确/词前缀/Alias/纠错/拼音各自有验证语义。
//! nucleo-matcher 只补充非连续对齐证据，不否决其他通道。

use fixedbitset::FixedBitSet;
use ib_pinyin::{matcher::PinyinMatcher, pinyin::PinyinNotation};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config as NucleoConfig, Matcher as NucleoMatcher, Utf32Str};

use crate::model::SearchResult;
use crate::search::alias;
use crate::search::fuzzy::fuzzy_match;
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
use crate::search::retrieval::query::{MixedPart, ParsedQuery};

/// 有序跳字命中分：低于 fuzzy(2)，避免压过真正的编辑距离命中。
pub const SCORE_SKIP: i32 = 320;
/// nucleo 非连续对齐分上限（低于 skip，只作补充证据）。
pub const SCORE_NUCLEO_MAX: i32 = 300;

const MIN_COMPACT_SUBSTR_LEN: usize = 3;
const MIN_WORD_PREFIX_LEN: usize = 2;
const MIN_ACRONYM_LEN: usize = 2;
/// 跳字最大额外跨度（Query 长度之外允许跳过的字符数）。
const SKIP_EXTRA_SPAN: usize = 3;

/// 命中字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchField {
    Name,
    Display,
    Alias,
    Pinyin,
    Keyword,
    Context,
}

/// 匹配方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Prefix,
    Contiguous,
    Acronym,
    Skip,
    Fuzzy,
}

/// 排序用匹配证据：同分时完整词/词边界/连续紧凑优先。
#[derive(Debug, Clone, Copy)]
pub struct MatchEvidence {
    pub field: MatchField,
    pub kind: MatchKind,
    /// 命中起点（字符）；拼音映射回原名称后的起点，未知为 usize::MAX。
    pub start: usize,
    /// 连续覆盖长度（字符）。
    pub span: usize,
    /// 非连续额外跳过数。
    pub gaps: usize,
    /// 编辑距离代价（fuzzy/symspell）。
    pub edit_cost: usize,
}

impl MatchEvidence {
    pub fn exact(field: MatchField) -> Self {
        Self {
            field,
            kind: MatchKind::Exact,
            start: 0,
            span: usize::MAX,
            gaps: 0,
            edit_cost: 0,
        }
    }

    pub fn contiguous(field: MatchField, start: usize, span: usize) -> Self {
        Self {
            field,
            kind: MatchKind::Contiguous,
            start,
            span,
            gaps: 0,
            edit_cost: 0,
        }
    }

    pub fn prefix(field: MatchField, span: usize) -> Self {
        Self {
            field,
            kind: MatchKind::Prefix,
            start: 0,
            span,
            gaps: 0,
            edit_cost: 0,
        }
    }

    /// 同分比较：更早起点、更少跳空、更低编辑代价、更长连续覆盖；再按字段与匹配方式稳定性。
    pub fn outranks(&self, other: &Self) -> bool {
        match (self.start, other.start) {
            (a, b) if a != b => a < b,
            _ => {
                if self.gaps != other.gaps {
                    return self.gaps < other.gaps;
                }
                if self.edit_cost != other.edit_cost {
                    return self.edit_cost < other.edit_cost;
                }
                if self.span != other.span {
                    return self.span > other.span;
                }
                let fr = field_rank(self.field).cmp(&field_rank(other.field));
                if fr != std::cmp::Ordering::Equal {
                    return fr == std::cmp::Ordering::Less;
                }
                kind_rank(self.kind) < kind_rank(other.kind)
            }
        }
    }
}

fn field_rank(field: MatchField) -> u8 {
    match field {
        MatchField::Name => 0,
        MatchField::Display => 1,
        MatchField::Alias => 2,
        MatchField::Pinyin => 3,
        MatchField::Keyword => 4,
        MatchField::Context => 5,
    }
}

fn kind_rank(kind: MatchKind) -> u8 {
    match kind {
        MatchKind::Exact => 0,
        MatchKind::Prefix => 1,
        MatchKind::Contiguous => 2,
        MatchKind::Acronym => 3,
        MatchKind::Skip => 4,
        MatchKind::Fuzzy => 5,
    }
}

#[derive(Debug, Clone)]
pub struct ScoredHit {
    pub doc_id: DocId,
    pub score: i32,
    pub matched_by: String,
    pub evidence: MatchEvidence,
}

/// 最终排序用的轻量候选：证据与稳定 id 保留到截断后再物化展示对象。
#[derive(Debug, Clone)]
pub struct RankedHit {
    pub doc_id: DocId,
    /// 最终分（含偏好加分后）。
    pub score: i32,
    pub quality_tier: i32,
    pub matched_by: String,
    pub evidence: MatchEvidence,
    pub name_len: usize,
    pub name_lower: String,
    pub stable_id: String,
}

/// 同分证据比较：更早起点 / 更少跳空 / 更低代价 / 更长覆盖优先。
pub fn cmp_evidence(a: &MatchEvidence, b: &MatchEvidence) -> std::cmp::Ordering {
    if a.outranks(b) {
        std::cmp::Ordering::Less
    } else if b.outranks(a) {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

/// 统一最终比较器：层 → 分 → 证据 → 名称长度 → 名称 → 稳定 id。
pub fn cmp_ranked_hit(a: &RankedHit, b: &RankedHit) -> std::cmp::Ordering {
    a.quality_tier
        .cmp(&b.quality_tier)
        .then_with(|| b.score.cmp(&a.score))
        .then_with(|| cmp_evidence(&a.evidence, &b.evidence))
        .then_with(|| a.name_len.cmp(&b.name_len))
        .then_with(|| a.name_lower.cmp(&b.name_lower))
        .then_with(|| a.stable_id.cmp(&b.stable_id))
}

/// 每 Query 构建一次的匹配上下文：ib-pinyin 混合拼音 + nucleo 对齐缓冲。
pub struct QueryContext<'a> {
    pub parsed: &'a ParsedQuery,
    pinyin: Option<PinyinMatcher<'a>>,
    nucleo: NucleoMatcher,
    nucleo_atom: Option<Atom>,
    hay_buf: Vec<char>,
}

impl<'a> QueryContext<'a> {
    pub fn build(q: &'a ParsedQuery, _index: &RetrievalIndex) -> Self {
        // 含 ASCII 即可参与拼音解释（混输：汉字+拼音/英文）；1 字符走首字母倒排，不建 matcher
        let pinyin = if q.has_ascii_alnum && q.chars.len() >= 2 {
            Some(
                PinyinMatcher::builder(q.raw_norm.as_str())
                    .pinyin_notations(PinyinNotation::Ascii | PinyinNotation::AsciiFirstLetter)
                    .is_pattern_partial(true)
                    .build(),
            )
        } else {
            None
        };
        let nucleo_atom = if q.chars.len() >= 2 {
            Some(Atom::new(
                q.raw_norm.as_str(),
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false,
            ))
        } else {
            None
        };
        Self {
            parsed: q,
            pinyin,
            nucleo: NucleoMatcher::new(NucleoConfig::DEFAULT),
            nucleo_atom,
            hay_buf: Vec::new(),
        }
    }
}

/// 字符位图：ASCII 用 fixedbitset，其余保留有序列表（量小）。
#[derive(Debug, Clone, Default)]
pub struct CharBits {
    ascii: FixedBitSet,
    other: Vec<char>,
}

impl CharBits {
    pub fn from_str(s: &str) -> Self {
        let mut ascii = FixedBitSet::with_capacity(128);
        let mut other: Vec<char> = Vec::new();
        for c in s.chars() {
            if (c as u32) < 128 {
                ascii.insert(c as usize);
            } else if !other.contains(&c) {
                other.push(c);
            }
        }
        other.sort_unstable();
        Self { ascii, other }
    }

    /// 必要条件：Query 每个字符都在集合中。不能视为命中。
    pub fn contains_all(&self, chars: &[char]) -> bool {
        chars.iter().all(|c| self.contains(*c))
    }

    fn contains(&self, c: char) -> bool {
        if (c as u32) < 128 {
            self.ascii.contains(c as usize)
        } else {
            self.other.binary_search(&c).is_ok()
        }
    }
}

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
        if let Some(gaps) = ordered_token_gaps(&q.tokens, &doc.tokens) {
            best = take_best(
                best,
                SCORE_TOKEN_SEQ,
                "token-seq",
                MatchEvidence {
                    field: MatchField::Name,
                    kind: MatchKind::Contiguous,
                    start: 0,
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
            if matcher.is_match(haystack.as_str()) {
                // 前缀式匹配（is_pattern_partial）略低于完整全拼；位置未知记 MAX，不当作最优起点
                best = take_best(
                    best,
                    SCORE_PINYIN_EXACT - 20,
                    "pinyin-ib",
                    MatchEvidence {
                        field: MatchField::Pinyin,
                        kind: MatchKind::Contiguous,
                        start: usize::MAX,
                        span: q_len,
                        gaps: 0,
                        edit_cost: 0,
                    },
                );
            }
        } else if !doc.pinyin_syllables.is_empty() && q_raw.chars().count() >= 2 {
            if let Some((score, kind)) = match_mixed_pinyin(&doc.pinyin_syllables, q_raw) {
                best = take_best(
                    best,
                    score,
                    kind,
                    MatchEvidence {
                        field: MatchField::Pinyin,
                        kind: MatchKind::Contiguous,
                        start: 0,
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
        if let Some((score, gaps)) = ordered_skip_score(q_raw, name) {
            best = take_best(
                best,
                score,
                "skip",
                MatchEvidence {
                    field: MatchField::Name,
                    kind: MatchKind::Skip,
                    start: 0,
                    span: q_len,
                    gaps,
                    edit_cost: 0,
                },
            );
        } else if let Some((score, gaps)) = ordered_skip_score(q_raw, display) {
            best = take_best(
                best,
                score,
                "skip",
                MatchEvidence {
                    field: MatchField::Display,
                    kind: MatchKind::Skip,
                    start: 0,
                    span: q_len,
                    gaps,
                    edit_cost: 0,
                },
            );
        } else {
            for (keyword, bits) in doc.keywords.iter().zip(&doc.keyword_bits) {
                if bits.contains_all(&q.chars) {
                    if let Some((score, gaps)) = ordered_skip_score(q_raw, keyword) {
                        best = take_best(
                            best,
                            score - 30,
                            "keyword-skip",
                            MatchEvidence {
                                field: MatchField::Keyword,
                                kind: MatchKind::Skip,
                                start: 0,
                                span: q_len,
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

    // 6) Fuzzy / SymSpell 验证（编辑距离）；1 字符不纠错
    if best.is_none() && q_raw.chars().count() >= 2 {
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
        }
    }

    best
}

/// 拼音首字母串中的字节偏移 → display 字符下标（每个非空白字符对应一位首字母）。
fn map_initial_index_to_display(doc: &IndexedDoc, initial_byte_at: usize) -> usize {
    let initials = doc.pinyin_initials.as_str();
    let idx = initials[..initial_byte_at].chars().count();
    doc.item
        .display_name
        .chars()
        .filter(|c| !c.is_whitespace())
        .enumerate()
        .find(|(i, _)| *i == idx)
        .map(|(i, _)| i)
        .unwrap_or(usize::MAX)
}

/// nucleo u16 分映射到本内核分数域（低质量，不抢精确/前缀）。
fn map_nucleo_score(raw: u16) -> i32 {
    // nucleo 分数量级约 0..~200+；线性压到 100..SCORE_NUCLEO_MAX
    let s = (raw as i32).clamp(0, 200);
    100 + (s * (SCORE_NUCLEO_MAX - 100)) / 200
}

fn fuzzy_name_or_tokens(q: &str, name: &str) -> Option<(i32, usize)> {
    if let Some(hit) = fuzzy_match(q, name) {
        return Some(hit);
    }
    name.split_whitespace()
        .filter_map(|tok| fuzzy_match(q, tok))
        .max_by_key(|(score, _)| *score)
}

/// Query 各词按序匹配名称词：整词、前缀，或连续词首字母串。
/// 返回跳过的名称词元数（gaps）。
fn ordered_token_gaps(q_tokens: &[String], n_tokens: &[String]) -> Option<usize> {
    if q_tokens.is_empty() || n_tokens.is_empty() {
        return None;
    }
    let mut ni = 0usize;
    let mut gaps = 0usize;
    for qt in q_tokens {
        let mut matched = false;
        while ni < n_tokens.len() {
            let nt = n_tokens[ni].as_str();
            if nt == qt || nt.starts_with(qt.as_str()) {
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
    Some(gaps)
}

/// 按 Query 混输段顺序对齐名称 + 拼音流。
/// 成功返回 (名称字符起点, 跳空, 名称覆盖跨度)。
fn ordered_mixed_align(
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
    let mut last_cjk = (0usize, 0usize);

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
                last_cjk = (start, name_i);
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
                // 刚匹配的 CJK 段拼音尾部复述（微信xin ← weixin 的 xin）
                if last_cjk.1 > last_cjk.0 {
                    let region_py: String = if syllables_for_name {
                        syllables[last_cjk.0..last_cjk.1].concat()
                    } else {
                        pinyin.to_string()
                    };
                    if region_py.ends_with(s.as_str()) || region_py.contains(s.as_str()) {
                        if first_start.is_none() {
                            first_start = Some(last_cjk.0);
                        }
                        continue;
                    }
                }
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
fn loose_mixed_fragments(q: &ParsedQuery, src: &str, doc: &IndexedDoc) -> bool {
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
/// 返回 (score, extra_gaps)。
fn ordered_skip_score(query: &str, text: &str) -> Option<(i32, usize)> {
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = text.chars().collect();
    if q.is_empty() || t.is_empty() || q.len() < 2 || q.len() > t.len() {
        return None;
    }
    let max_span = q.len() + SKIP_EXTRA_SPAN;
    // 对每个起点找最早终点，取最小跨度窗口
    let mut best_span = usize::MAX;
    for start in 0..t.len() {
        let mut qi = 0usize;
        for ti in start..t.len() {
            if t[ti] == q[qi] {
                qi += 1;
                if qi == q.len() {
                    let span = ti - start + 1;
                    if span < best_span {
                        best_span = span;
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
    Some((score.clamp(fuzzy_score(2), SCORE_SKIP), gap))
}

/// 混合全拼/简拼：音节序列上匹配 query。
/// 例：["wei","xin"] 可命中 weixin / wx / wxin / weix。
fn match_mixed_pinyin(syllables: &[String], query: &str) -> Option<(i32, &'static str)> {
    if syllables.is_empty() || query.is_empty() {
        return None;
    }
    let mut rest = query;
    let mut used_full = 0usize;
    let mut used_init = 0usize;
    for syl in syllables {
        if rest.is_empty() {
            break;
        }
        if let Some(rem) = rest.strip_prefix(syl.as_str()) {
            used_full += 1;
            rest = rem;
            continue;
        }
        if let Some(first) = syl.chars().next() {
            if rest.starts_with(first) {
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
    if used_init == 0 && used_full == syllables.len() {
        Some((SCORE_PINYIN_EXACT, "pinyin-mixed-exact"))
    } else if used_full > 0 {
        Some((SCORE_PINYIN_EXACT - 30, "pinyin-mixed"))
    } else {
        Some((SCORE_PINYIN_INITIAL, "pinyin-mixed-initial"))
    }
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
    use super::*;

    #[test]
    fn skip_matches_google_chrome() {
        // googchrome 跳过 google 中的 l、e 与空格，按序命中
        let hit = ordered_skip_score("googchrome", "google chrome");
        assert!(hit.is_some(), "googchrome 应有序命中 google chrome");
        assert!(hit.unwrap().0 <= SCORE_SKIP);
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
        let hit = match_mixed_pinyin(&syl, "wxin");
        assert!(hit.is_some(), "wxin 应命中 微信 音节");
        let hit = match_mixed_pinyin(&syl, "weixin");
        assert!(hit.is_some());
        let hit = match_mixed_pinyin(&syl, "wx");
        assert!(hit.is_some());
        assert!(match_mixed_pinyin(&syl, "zzz").is_none());
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
    fn ordered_mixed_accepts_name_then_pinyin_suffix() {
        let parts = vec![
            MixedPart::Cjk("微信".into()),
            MixedPart::Latin("xin".into()),
        ];
        let syl = vec!["wei".to_string(), "xin".to_string()];
        let hit = ordered_mixed_align(&parts, "微信", "weixin", &syl);
        assert!(hit.is_some(), "微信xin 应有序对齐到微信");
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
        assert!(ordered_token_gaps(&q, &n).is_some());
        let wrong: Vec<String> = tokens("code visual")
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        assert!(ordered_token_gaps(&wrong, &n).is_none());
    }
}
