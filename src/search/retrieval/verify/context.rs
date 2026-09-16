//! 每 Query 的匹配上下文与字符位图。
//!
//! 职责：持有 ib-pinyin / nucleo 工作缓冲，避免每候选重建。

use fixedbitset::FixedBitSet;
use ib_pinyin::{matcher::PinyinMatcher, pinyin::PinyinNotation};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config as NucleoConfig, Matcher as NucleoMatcher};

use crate::search::retrieval::query::ParsedQuery;

/// 跨查询复用的匹配工作区。
///
/// `NucleoMatcher` 与对齐缓冲的初始化成本与查询无关：常驻 worker 持有它、
/// 跨查询复用（每次查询只更新 pattern 与解析结果）；一次性搜索入口用完即弃。
pub struct MatcherScratch {
    pub(crate) nucleo: NucleoMatcher,
    pub(crate) hay_buf: Vec<char>,
}

impl Default for MatcherScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl MatcherScratch {
    pub fn new() -> Self {
        Self {
            nucleo: NucleoMatcher::new(NucleoConfig::DEFAULT),
            hay_buf: Vec::new(),
        }
    }
}

/// 每 Query 构建一次的匹配上下文：ib-pinyin 混合拼音 + 借用来的 nucleo 工作区。
pub struct QueryContext<'a> {
    pub parsed: &'a ParsedQuery,
    pub(crate) pinyin: Option<PinyinMatcher<'a>>,
    pub(crate) nucleo_atom: Option<Atom>,
    pub(crate) scratch: &'a mut MatcherScratch,
}

impl<'a> QueryContext<'a> {
    pub fn build(q: &'a ParsedQuery, scratch: &'a mut MatcherScratch) -> Self {
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
            nucleo_atom,
            scratch,
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


