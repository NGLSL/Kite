//! 历史加权：把 Usage / Recency / Query History 映射为排序加分。
//! 原则（PRD §41–42）：
//! - MatchScore 仍是主信号，先按基础相关性分层
//! - 历史只在同层（或相近质量）候选间调整顺序，不得把无关弱匹配推到明确匹配之上
//! - 历史总加分有上限；不做纯 LRU

use std::collections::{HashMap, HashSet};

use crate::model::SearchResult;
use crate::storage::{QueryPairStats, UsageStats};

/// 历史总加分上限。须 < (SCORE_NAME_EXACT - SCORE_PREFIX) = 200。
pub const HISTORY_BOOST_MAX: i32 = 160;

/// 固定项加分。与历史取较大者，须满足：固定 Prefix(800+180) 仍低于 Name Exact(1000)
/// （明确匹配保护，见 PRD §41–42）。
pub const PIN_BOOST: i32 = 180;

const FREQUENCY_CAP: i32 = 50;
const RECENCY_CAP: i32 = 40;

/// 基础相关性层级：数值越小质量越高。历史只在同层内调整顺序。
pub fn quality_tier(base_score: i32) -> i32 {
    use crate::search::ranker::{
        SCORE_ACRONYM, SCORE_BUILTIN_ALIAS_EXACT, SCORE_COMPACT_EXACT, SCORE_COMPACT_SUBSTRING,
        SCORE_FUZZY_MAX, SCORE_NAME_EXACT, SCORE_PINYIN_EXACT, SCORE_PINYIN_INITIAL, SCORE_PREFIX,
        SCORE_SUBSTRING, SCORE_TOKEN_SEQ, SCORE_USER_ALIAS_EXACT, SCORE_WORD_EXACT,
        SCORE_WORD_PREFIX,
    };
    if base_score >= SCORE_USER_ALIAS_EXACT {
        0
    } else if base_score >= SCORE_NAME_EXACT {
        1
    } else if base_score >= SCORE_BUILTIN_ALIAS_EXACT || base_score >= SCORE_COMPACT_EXACT {
        2
    } else if base_score >= SCORE_TOKEN_SEQ || base_score >= SCORE_WORD_EXACT {
        3
    } else if base_score >= SCORE_PREFIX {
        4
    } else if base_score >= SCORE_COMPACT_SUBSTRING
        || base_score >= SCORE_PINYIN_EXACT
        || base_score >= SCORE_PINYIN_INITIAL
    {
        5
    } else if base_score >= SCORE_WORD_PREFIX || base_score >= SCORE_ACRONYM {
        6
    } else if base_score >= SCORE_SUBSTRING {
        7
    } else if base_score >= SCORE_FUZZY_MAX / 2 {
        8
    } else {
        9
    }
}

/// 个性化加分统一入口（Match 仍是主信号）：历史（Usage/Recency/Query 配对）+ 固定。
/// 先按 base score 分层，再在层内用历史/Pin 调整顺序，最后按 (tier, score) 重排。
pub fn apply_boosts(
    hits: &mut [SearchResult],
    usage: HashMap<String, UsageStats>,
    pairs: HashMap<String, QueryPairStats>,
    query_norm: &str,
    now: i64,
    pinned: &HashSet<String>,
) {
    let mut keyed: Vec<(i32, i32, SearchResult)> = Vec::with_capacity(hits.len());
    for hit in hits.iter() {
        let base = hit.score;
        let tier = quality_tier(base);
        let is_pinned = pinned.contains(&hit.item.id);
        let u = usage.get(&hit.item.id).cloned().unwrap_or_default();
        let p = pairs.get(&hit.item.id).cloned().unwrap_or_default();
        let history = history_boost(query_norm, &u, &p, now);
        let boost = if is_pinned { PIN_BOOST.max(history) } else { history };
        let mut hit = hit.clone();
        hit.score = base + boost;
        if is_pinned {
            hit.matched_by = format!("{}+pin", hit.matched_by);
        } else if boost > 0 {
            hit.matched_by = format!("{}+history", hit.matched_by);
        }
        keyed.push((tier, hit.score, hit));
    }
    // 同层内按加分后分数降序；层号小 = 质量高，优先
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    for (slot, (_, _, hit)) in hits.iter_mut().zip(keyed.into_iter()) {
        *slot = hit;
    }
}

/// 计算历史加分（已 clamp 到 HISTORY_BOOST_MAX）。`_query` 预留调试。
pub fn history_boost(_query: &str, usage: &UsageStats, pair: &QueryPairStats, now: i64) -> i32 {
    let q = query_pair_score(pair);
    let f = frequency_score(usage.launch_count);
    let r = recency_score(usage.last_used_at, now);
    (q + f + r).min(HISTORY_BOOST_MAX)
}

/// Query→App 配对越稳越高；对数增长，单独封顶 90。
fn query_pair_score(pair: &QueryPairStats) -> i32 {
    if pair.count <= 0 {
        return 0;
    }
    // 1→20, 3→38, 10→60, 30→~85
    let s = 20.0 + 15.0 * (pair.count as f64).ln();
    (s.round() as i32).min(90)
}

fn frequency_score(count: i64) -> i32 {
    if count <= 0 {
        return 0;
    }
    let s = 10.0 + 8.0 * (count as f64).ln();
    (s.round() as i32).min(FREQUENCY_CAP)
}

/// 最近使用适量加分；7 天内衰减，避免「刚点一次就顶掉常用」。
fn recency_score(last_used_at: i64, now: i64) -> i32 {
    if last_used_at <= 0 || now <= last_used_at {
        return 0;
    }
    let age_days = (now - last_used_at) as f64 / 86_400.0;
    if age_days < 1.0 {
        RECENCY_CAP
    } else if age_days < 7.0 {
        // 1 天满分 → 7 天约 0
        let t = (7.0 - age_days) / 6.0;
        (RECENCY_CAP as f64 * t).round() as i32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AppItem;

    fn hit(id: &str) -> SearchResult {
        SearchResult {
            item: AppItem::scanned(id.into(), id.into(), format!("C:\\{id}.exe"), None, None, "t"),
            score: 100,
            matched_by: "test".into(),
        }
    }

    #[test]
    fn apply_boosts_missing_rows_are_zero() {
        let mut hits = vec![hit("a"), hit("b")];
        apply_boosts(&mut hits, HashMap::new(), HashMap::new(), "q", 1, &HashSet::new());
        assert_eq!(hits[0].score, 100, "无历史记录 → 不加分");
        assert_eq!(hits[0].matched_by, "test");
    }

    #[test]
    fn apply_boosts_marks_history() {
        let mut hits = vec![hit("a")];
        let mut usage = HashMap::new();
        usage.insert(
            "a".to_string(),
            UsageStats {
                launch_count: 10,
                last_used_at: 1_000,
            },
        );
        apply_boosts(&mut hits, usage, HashMap::new(), "q", 2_000, &HashSet::new());
        assert!(hits[0].score > 100);
        assert!(hits[0].matched_by.ends_with("+history"));
    }

    #[test]
    fn boost_capped() {
        let usage = UsageStats {
            launch_count: 10_000,
            last_used_at: 0,
        };
        let pair = QueryPairStats {
            count: 10_000,
            last_used_at: 0,
        };
        let b = history_boost("wx", &usage, &pair, 1_000_000);
        assert!(b <= HISTORY_BOOST_MAX);
    }

    #[test]
    fn pair_dominates_frequency() {
        let usage = UsageStats {
            launch_count: 1,
            last_used_at: 0,
        };
        let strong = QueryPairStats {
            count: 20,
            last_used_at: 0,
        };
        let weak = QueryPairStats::default();
        assert!(
            history_boost("wx", &usage, &strong, 0)
                > history_boost("wx", &usage, &weak, 0)
        );
    }

    #[test]
    fn recency_small() {
        let now = 1_700_000_000;
        let fresh = UsageStats {
            launch_count: 1,
            last_used_at: now - 3600,
        };
        let old = UsageStats {
            launch_count: 1,
            last_used_at: now - 30 * 86_400,
        };
        assert!(
            recency_score(fresh.last_used_at, now) > recency_score(old.last_used_at, now)
        );
    }

    #[test]
    fn pin_boost_ranks_pinned_item_up() {
        let mut hits = vec![hit("a"), hit("b")];
        let pinned: HashSet<String> = ["b".to_string()].into_iter().collect();
        apply_boosts(&mut hits, HashMap::new(), HashMap::new(), "q", 1, &pinned);
        assert_eq!(hits[0].item.id, "b", "固定项在同层内应排到前面");
        assert_eq!(hits[0].score, 100 + PIN_BOOST, "无历史时固定项获得 PIN_BOOST");
        assert!(hits[0].matched_by.ends_with("+pin"));
        assert_eq!(hits[1].item.id, "a");
        assert_eq!(hits[1].score, 100, "未固定不加分");
    }

    #[test]
    fn pin_does_not_stack_with_history() {
        // 固定与历史取较大者：两者都有时不得叠加突破明确匹配保护
        let mut hits = vec![hit("a")];
        let mut usage = HashMap::new();
        usage.insert(
            "a".to_string(),
            UsageStats {
                launch_count: 10_000,
                last_used_at: 0,
            },
        );
        let pinned: HashSet<String> = ["a".to_string()].into_iter().collect();
        apply_boosts(&mut hits, usage, HashMap::new(), "q", 1, &pinned);
        assert!(
            hits[0].score <= 100 + PIN_BOOST,
            "固定+历史叠加后得分 {} 超过上限", hits[0].score
        );
    }

    #[test]
    fn pin_boost_keeps_exact_above_pinned_prefix() {
        // 明确匹配保护：固定项的 Prefix 加成不得压过另一条的 Name Exact
        let pinned_prefix = crate::search::ranker::SCORE_PREFIX + PIN_BOOST;
        let exact = crate::search::ranker::SCORE_NAME_EXACT;
        assert!(exact > pinned_prefix);
    }

    #[test]
    fn pin_boost_empty_set_noop() {
        let mut hits = vec![hit("a")];
        apply_boosts(&mut hits, HashMap::new(), HashMap::new(), "q", 1, &HashSet::new());
        assert_eq!(hits[0].score, 100);
        assert_eq!(hits[0].matched_by, "test");
    }

    #[test]
    fn history_cannot_lift_substring_above_word_prefix() {
        use crate::search::ranker::{SCORE_SUBSTRING, SCORE_WORD_PREFIX};
        let mut weak = hit("weak");
        weak.score = SCORE_SUBSTRING;
        let mut strong = hit("strong");
        strong.score = SCORE_WORD_PREFIX;
        let mut usage = HashMap::new();
        usage.insert(
            "weak".to_string(),
            UsageStats {
                launch_count: 50,
                last_used_at: 1_700_000_000,
            },
        );
        let mut pairs = HashMap::new();
        pairs.insert(
            "weak".to_string(),
            QueryPairStats {
                count: 30,
                last_used_at: 1_700_000_000,
            },
        );
        let mut hits = vec![weak, strong];
        apply_boosts(&mut hits, usage, pairs, "ter", 1_700_086_400, &HashSet::new());
        assert_eq!(
            hits[0].item.id, "strong",
            "更高基础相关性的 word-prefix 不得被弱 substring+历史压过"
        );
    }

    #[test]
    fn history_reorders_within_same_tier() {
        use crate::search::ranker::SCORE_WORD_PREFIX;
        let mut a = hit("xterminal");
        a.score = SCORE_WORD_PREFIX;
        let mut b = hit("terminal");
        b.score = SCORE_WORD_PREFIX;
        let mut usage = HashMap::new();
        usage.insert(
            "xterminal".to_string(),
            UsageStats {
                launch_count: 8,
                last_used_at: 1_700_000_000,
            },
        );
        let mut pairs = HashMap::new();
        pairs.insert(
            "xterminal".to_string(),
            QueryPairStats {
                count: 5,
                last_used_at: 1_700_000_000,
            },
        );
        let mut hits = vec![b, a];
        apply_boosts(&mut hits, usage, pairs, "ter", 1_700_086_400, &HashSet::new());
        assert_eq!(hits[0].item.id, "xterminal", "同层内历史应把更常用项提到前面");
    }

    #[test]
    fn quality_tier_orders_exact_before_fuzzy() {
        use crate::search::ranker::{SCORE_FUZZY_MAX, SCORE_NAME_EXACT, SCORE_PREFIX, SCORE_SUBSTRING};
        assert!(quality_tier(SCORE_NAME_EXACT) < quality_tier(SCORE_PREFIX));
        assert!(quality_tier(SCORE_PREFIX) < quality_tier(SCORE_SUBSTRING));
        assert!(quality_tier(SCORE_SUBSTRING) < quality_tier(SCORE_FUZZY_MAX / 2));
    }
}
