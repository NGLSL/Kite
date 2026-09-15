//! 匹配证据与最终排序比较器。
//!
//! 职责：声明命中字段/方式、位置证据，以及同分细排与统一最终比较。

use super::DocId;

/// 有序跳字命中分：低于 fuzzy(2)，避免压过真正的编辑距离命中。
pub const SCORE_SKIP: i32 = 320;
/// nucleo 非连续对齐分上限（低于 skip，只作补充证据）。
pub const SCORE_NUCLEO_MAX: i32 = 300;

// Re-export type definitions below via include of original body.
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
    pub source: String,
    /// 是否已有可展示图标；归并时优先用带图标的入口做代表。
    pub has_icon: bool,
    /// 图标可来自组内另一入口（启动仍用 doc_id 对应的 target/args）。
    pub icon_doc_id: DocId,
    /// 快捷方式通常带 working_dir；同源时优先作为启动代表。
    pub has_working_dir: bool,
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

/// 统一最终比较器：
/// 明确匹配保护组（Alias/Name Exact 等）始终在前，组内按 层→分→证据；
/// 普通组不按小层锁死，按 最终分→证据→稳定兜底，允许偏好抬升相近候选。
pub fn cmp_ranked_hit(a: &RankedHit, b: &RankedHit) -> std::cmp::Ordering {
    use crate::history::PROTECTED_TIER_MAX;
    let a_protected = a.quality_tier <= PROTECTED_TIER_MAX;
    let b_protected = b.quality_tier <= PROTECTED_TIER_MAX;
    let stable = |a: &RankedHit, b: &RankedHit| {
        a.name_len
            .cmp(&b.name_len)
            .then_with(|| a.name_lower.cmp(&b.name_lower))
            .then_with(|| a.stable_id.cmp(&b.stable_id))
    };
    match (a_protected, b_protected) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (true, true) => a
            .quality_tier
            .cmp(&b.quality_tier)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| cmp_evidence(&a.evidence, &b.evidence))
            .then_with(|| stable(a, b)),
        (false, false) => b
            .score
            .cmp(&a.score)
            .then_with(|| cmp_evidence(&a.evidence, &b.evidence))
            .then_with(|| stable(a, b)),
    }
}


