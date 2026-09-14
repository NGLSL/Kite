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
    fuzzy_score, SCORE_ACRONYM, SCORE_BUILTIN_ALIAS_EXACT, SCORE_COMPACT_EXACT,
    SCORE_COMPACT_SUBSTRING, SCORE_NAME_EXACT, SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL,
    SCORE_PINYIN_INITIAL_INNER, SCORE_PREFIX, SCORE_SUBSTRING, SCORE_TOKEN_SEQ,
    SCORE_USER_ALIAS_EXACT, SCORE_WORD_EXACT, SCORE_WORD_PREFIX,
};
use crate::search::retrieval::channels::Candidates;
use crate::search::retrieval::doc::{DocId, IndexedDoc, RetrievalIndex};
use crate::search::retrieval::query::ParsedQuery;

/// 有序跳字命中分：低于 fuzzy(2)，避免压过真正的编辑距离命中。
pub const SCORE_SKIP: i32 = 320;
/// nucleo 非连续对齐分上限（低于 skip，只作补充证据）。
pub const SCORE_NUCLEO_MAX: i32 = 300;

const MIN_COMPACT_SUBSTR_LEN: usize = 3;
const MIN_WORD_PREFIX_LEN: usize = 2;
const MIN_ACRONYM_LEN: usize = 2;
/// 跳字最大额外跨度（Query 长度之外允许跳过的字符数）。
const SKIP_EXTRA_SPAN: usize = 3;

#[derive(Debug, Clone)]
pub struct ScoredHit {
    pub doc_id: DocId,
    pub score: i32,
    pub matched_by: String,
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
        let pinyin = if q.latin && q.chars.len() >= 2 {
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
    for &id in &candidates.ids {
        let Some(doc) = index.doc(id) else { continue };
        if let Some((score, matched_by)) = verify_one(doc, ctx, user_targets) {
            hits.push(ScoredHit {
                doc_id: id,
                score,
                matched_by,
            });
        }
    }
    hits
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

fn verify_one(
    doc: &IndexedDoc,
    ctx: &mut QueryContext,
    user_targets: &[UserTarget],
) -> Option<(i32, String)> {
    let q = ctx.parsed;
    let name = doc.name.as_str();
    let display = doc.display.as_str();
    let q_raw = q.raw_norm.as_str();
    let q_compact = q.compact.as_str();

    let mut best: Option<(i32, &'static str)> = None;

    // 0) 用户 Alias
    let hit_user = user_targets.iter().any(|t| match &t.id {
        Some(id) => doc.item.id == *id,
        None => {
            let n = t.name.to_lowercase();
            name == n.as_str() || name.contains(n.as_str()) || display.contains(n.as_str())
        }
    });
    if hit_user {
        best = take_best(best, SCORE_USER_ALIAS_EXACT, "user-alias-exact");
    }

    // 1) 内置 Alias
    if let Some(fragments) = alias::targets_for(q_raw) {
        if fragments
            .iter()
            .any(|t| name.contains(t) || display.contains(t))
        {
            best = take_best(best, SCORE_BUILTIN_ALIAS_EXACT, "alias-exact");
        }
    }

    // 2) Exact / Prefix / Substring
    for candidate in [name, display] {
        if candidate == q_raw {
            best = take_best(best, SCORE_NAME_EXACT, "exact");
        } else if candidate.starts_with(q_raw) {
            best = take_best(best, SCORE_PREFIX, "prefix");
        } else if candidate.contains(q_raw) {
            best = take_best(best, SCORE_SUBSTRING, "substring");
        }
    }

    // 2b) Compact
    if q_compact.len() >= 2 {
        for candidate in [doc.compact_name.as_str(), doc.compact_display.as_str()] {
            if candidate == q_compact {
                best = take_best(best, SCORE_COMPACT_EXACT, "compact-exact");
            } else if q_compact.len() >= MIN_COMPACT_SUBSTR_LEN && candidate.contains(q_compact) {
                best = take_best(best, SCORE_COMPACT_SUBSTRING, "compact-substring");
            }
        }
    }

    // 2c) 词边界
    for tok in &doc.tokens {
        if tok == q_raw {
            best = take_best(best, SCORE_WORD_EXACT, "word-exact");
        } else if q_raw.chars().count() >= MIN_WORD_PREFIX_LEN && tok.starts_with(q_raw) {
            best = take_best(best, SCORE_WORD_PREFIX, "word-prefix");
        }
    }

    // 2d) 有序多词
    if q.tokens.len() >= 2 && ordered_token_match(&q.tokens, &doc.tokens) {
        best = take_best(best, SCORE_TOKEN_SEQ, "token-seq");
    }

    // 2e) 英文缩写
    if q_raw.chars().count() >= MIN_ACRONYM_LEN && !doc.acronym.is_empty() && doc.acronym == q_raw {
        best = take_best(best, SCORE_ACRONYM, "acronym");
    }

    // 3) 拼音（全拼 / 首字母 / ib-pinyin 混合+多音字）
    if !doc.pinyin.is_empty() {
        if doc.pinyin == q_raw {
            best = take_best(best, SCORE_PINYIN_EXACT, "pinyin-exact");
        } else if doc.pinyin.starts_with(q_raw) {
            best = take_best(best, SCORE_PINYIN_EXACT - 50, "pinyin-prefix");
        }
    }
    if !doc.pinyin_initials.is_empty() {
        if doc.pinyin_initials == q_raw {
            best = take_best(best, SCORE_PINYIN_INITIAL, "pinyin-initial");
        } else if q_raw.chars().count() >= 2 && doc.pinyin_initials.starts_with(q_raw) {
            best = take_best(best, SCORE_PINYIN_INITIAL - 40, "pinyin-initial-prefix");
        } else if q_raw.chars().count() >= 2 && doc.pinyin_initials.contains(q_raw) {
            best = take_best(best, SCORE_PINYIN_INITIAL_INNER, "pinyin-initial-inner");
        }
    }
    // ib-pinyin：混合全拼/简拼/多音字（对原文匹配，覆盖非默认读音）
    if q.latin {
        if let Some(matcher) = ctx.pinyin.as_ref() {
            let haystack = &doc.item.display_name;
            if matcher.is_match(haystack.as_str()) {
                // 前缀式匹配（is_pattern_partial）略低于完整全拼
                best = take_best(best, SCORE_PINYIN_EXACT - 20, "pinyin-ib");
            }
        } else if !doc.pinyin_syllables.is_empty() {
            if let Some((score, kind)) = match_mixed_pinyin(&doc.pinyin_syllables, q_raw) {
                best = take_best(best, score, kind);
            }
        }
    }

    // 4) 有序跳字（省略中间字符）；短 Query 选择性差，不走
    if best.map(|(s, _)| s < SCORE_SKIP).unwrap_or(true) && q_raw.chars().count() >= 4 {
        if let Some(score) = ordered_skip_score(q_raw, name) {
            best = take_best(best, score, "skip");
        } else if let Some(score) = ordered_skip_score(q_raw, display) {
            best = take_best(best, score, "skip");
        }
    }

    // 5) nucleo-matcher 非连续对齐：只补充证据，不否决已有命中
    if best.map(|(s, _)| s < SCORE_NUCLEO_MAX).unwrap_or(true) {
        if ctx.nucleo_atom.is_some() {
            let hay = Utf32Str::new(name, &mut ctx.hay_buf);
            let atom = ctx.nucleo_atom.as_ref().unwrap();
            if let Some(raw) = atom.score(hay, &mut ctx.nucleo) {
                let mapped = map_nucleo_score(raw);
                if mapped > 0 {
                    best = take_best(best, mapped, "nucleo");
                }
            }
        }
    }

    // 6) Fuzzy / SymSpell 验证（编辑距离）
    if best.is_none() {
        if let Some((score, _)) = fuzzy_name_or_tokens(q_raw, name)
            .or_else(|| fuzzy_name_or_tokens(q_raw, display))
        {
            best = take_best(best, score, "fuzzy");
        }
    }

    let (score, matched_by) = best?;
    Some((score, matched_by.to_string()))
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
fn ordered_token_match(q_tokens: &[String], n_tokens: &[String]) -> bool {
    if q_tokens.is_empty() || n_tokens.is_empty() {
        return false;
    }
    let mut ni = 0usize;
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
        }
        if !matched {
            return false;
        }
    }
    true
}

/// 有序子序列命中：字符按序出现，跨度与词边界受控。
fn ordered_skip_score(query: &str, text: &str) -> Option<i32> {
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
    Some(score.clamp(fuzzy_score(2), SCORE_SKIP))
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
        if let Some((score, matched_by)) = verify_one(doc, &mut ctx, user_targets) {
            hits.push((
                score,
                doc.item.name.chars().count(),
                doc.item.name.to_lowercase(),
                doc.id,
                matched_by,
            ));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));
    hits.truncate(max_results);
    hits
        .into_iter()
        .map(|(score, _, _, id, matched_by)| {
            let doc = docs.iter().find(|d| d.id == id).unwrap();
            SearchResult {
                item: doc.item.clone(),
                score,
                matched_by,
            }
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
        assert!(hit.unwrap() <= SCORE_SKIP);
    }

    #[test]
    fn skip_rejects_wrong_order() {
        assert!(ordered_skip_score("emorhclod", "google chrome").is_none());
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
        let q = tokens("visual code");
        let n: Vec<String> = tokens("visual studio code").iter().map(|s| s.to_string()).collect();
        assert!(ordered_token_match(&q.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &n));
        let wrong = tokens("code visual")
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        assert!(!ordered_token_match(&wrong, &n));
    }
}
