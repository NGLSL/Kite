//! 历史加权：把 Usage / Recency / Query History 映射为排序加分。
//! 原则（PRD §41–42 / 票 04）：
//! - MatchScore 仍是主信号；明确匹配（用户 Alias / Name Exact）硬保护
//! - 本 Query 配对优先于全局 Usage；一次选择有限倾向，重复选择对数趋稳
//! - 相近质量候选可竞争，不把每种匹配方式锁成不可跨越的小层
//! - 历史总加分有上限；Pin 与历史取较大者；不做纯 LRU

use std::collections::{HashMap, HashSet};

use crate::model::SearchResult;
use crate::storage::{QueryPairStats, UsageStats};

/// 历史总加分上限。须 < (SCORE_NAME_EXACT - SCORE_PREFIX) = 200。
pub const HISTORY_BOOST_MAX: i32 = 160;

/// 固定项加分。与历史取较大者，须满足：固定 Prefix(800+180) 仍低于 Name Exact(1000)
/// （明确匹配保护，见 PRD §41–42）。
pub const PIN_BOOST: i32 = 180;

/// 硬保护层：≤ 此层的候选始终排在其余候选之前，且层内仍按 (tier, FinalScore)。
/// 用户 Alias / Name Exact / 内置 Alias / Compact Exact；
/// 其余层允许相近质量用 FinalScore 竞争。
pub const PROTECTED_TIER_MAX: i32 = 2;

const FREQUENCY_CAP: i32 = 45;
const RECENCY_CAP: i32 = 40;
/// Query 配对单独封顶：须明显高于单次 Usage，低于总上限，给 Recency 留空间。
const PAIR_CAP: i32 = 110;

/// 基础相关性层级：数值越小质量越高。
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

/// 一次检索使用的个性化状态（内存快照，截断前参与最终排序）。
#[derive(Debug, Default, Clone)]
pub struct Personalization {
    pub usage: HashMap<String, UsageStats>,
    pub pairs: HashMap<String, QueryPairStats>,
    pub pinned: HashSet<String>,
    /// 用户降权标记：非保护层候选扣分后移，可恢复。
    pub demoted: HashSet<String>,
    pub now: i64,
    pub query_norm: String,
}

/// 个性化加分统一入口（Match 仍是主信号）：历史（Usage/Recency/Query 配对）+ 固定。
pub fn apply_boosts(
    hits: &mut [SearchResult],
    usage: HashMap<String, UsageStats>,
    pairs: HashMap<String, QueryPairStats>,
    query_norm: &str,
    now: i64,
    pinned: &HashSet<String>,
) {
    let prefs = Personalization {
        usage,
        pairs,
        pinned: pinned.clone(),
        demoted: HashSet::new(),
        now,
        query_norm: query_norm.to_string(),
    };
    apply_personalization(hits, &prefs);
}

/// 个性化调整：由偏好快照统一计算，供 RankedHit / SearchResult 两条路径共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreferenceAdjust {
    /// 加到基础分上的调整（可为负：降权）。
    pub boost: i32,
    pub pinned: bool,
    pub demoted: bool,
    /// 是否应打 +history 标签（非 pin/demote 且 boost>0）。
    pub history: bool,
}

/// 单一计算入口：查询偏好 + 使用 + Pin + 降权 + 时间 → boost 与标记。
/// tier 用于判断是否处于明确匹配保护层（保护层不受降权扣分）。
pub fn preference_adjust(
    prefs: &Personalization,
    item_id: &str,
    quality_tier: i32,
) -> PreferenceAdjust {
    let pinned = prefs.pinned.contains(item_id);
    let demoted = prefs.demoted.contains(item_id);
    let u = prefs.usage.get(item_id).cloned().unwrap_or_default();
    let p = prefs.pairs.get(item_id).cloned().unwrap_or_default();
    let history = history_boost(&prefs.query_norm, &u, &p, prefs.now);
    let mut boost = if pinned {
        PIN_BOOST.max(history)
    } else {
        history
    };
    if demoted && quality_tier > PROTECTED_TIER_MAX {
        boost -= crate::storage::demote::DEMOTE_PENALTY;
    }
    let history = !pinned && !demoted && boost > 0;
    PreferenceAdjust {
        boost,
        pinned,
        demoted,
        history,
    }
}

/// 把偏好标记追加到 matched_by。
pub fn apply_preference_tags(matched_by: &str, adj: &PreferenceAdjust) -> String {
    let mut out = matched_by.to_string();
    if adj.pinned {
        out.push_str("+pin");
    }
    if adj.demoted {
        out.push_str("+demote");
    } else if adj.history {
        out.push_str("+history");
    }
    out
}

/// 在完整候选集上应用个性化并最终排序。调用方负责截断。
///
/// 排序：
/// 1. 硬保护层（Alias / Name Exact）始终在前，层内 (tier, FinalScore)
/// 2. 其余候选按 FinalScore 竞争（允许相近质量被本 Query 偏好调整）
/// 3. 同分再按 tier、id 稳定
pub fn apply_personalization(hits: &mut [SearchResult], prefs: &Personalization) {
    let mut protected: Vec<(i32, i32, String, SearchResult)> = Vec::new();
    let mut open: Vec<(i32, i32, String, SearchResult)> = Vec::new();

    for hit in hits.iter() {
        let base = hit.score;
        let tier = if hit.quality_tier != 0 {
            hit.quality_tier
        } else {
            quality_tier(base)
        };
        let adj = preference_adjust(prefs, &hit.item.id, tier);
        let mut hit = hit.clone();
        hit.quality_tier = tier;
        hit.score = base + adj.boost;
        hit.matched_by = apply_preference_tags(&hit.matched_by, &adj);
        let keyed = (tier, hit.score, hit.item.id.clone(), hit);
        if tier <= PROTECTED_TIER_MAX {
            protected.push(keyed);
        } else {
            open.push(keyed);
        }
    }

    protected.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
    open.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.2.cmp(&b.2))
    });

    let mut ordered = protected;
    ordered.extend(open);
    for (slot, (_, _, _, hit)) in hits.iter_mut().zip(ordered.into_iter()) {
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

/// Query→App 配对：本查询的明确选择，优先于全局 Usage。
/// 1→30, 3→45, 10→62, 30→~78；封顶 PAIR_CAP。
fn query_pair_score(pair: &QueryPairStats) -> i32 {
    if pair.count <= 0 {
        return 0;
    }
    let s = 30.0 + 14.0 * (pair.count as f64).ln();
    (s.round() as i32).min(PAIR_CAP)
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
        SearchResult::scored(
            AppItem::scanned(
                id.into(),
                id.into(),
                format!("C:\\{id}.exe"),
                None,
                None,
                "t",
            ),
            100,
            "test",
        )
    }

    #[test]
    fn apply_boosts_missing_rows_are_zero() {
        let mut hits = vec![hit("a"), hit("b")];
        apply_boosts(
            &mut hits,
            HashMap::new(),
            HashMap::new(),
            "q",
            1,
            &HashSet::new(),
        );
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
        apply_boosts(
            &mut hits,
            usage,
            HashMap::new(),
            "q",
            2_000,
            &HashSet::new(),
        );
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
        assert!(history_boost("wx", &usage, &strong, 0) > history_boost("wx", &usage, &weak, 0));
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
        assert!(recency_score(fresh.last_used_at, now) > recency_score(old.last_used_at, now));
    }

    #[test]
    fn pin_boost_ranks_pinned_item_up() {
        let mut hits = vec![hit("a"), hit("b")];
        let pinned: HashSet<String> = ["b".to_string()].into_iter().collect();
        apply_boosts(&mut hits, HashMap::new(), HashMap::new(), "q", 1, &pinned);
        assert_eq!(hits[0].item.id, "b", "固定项在同层内应排到前面");
        assert_eq!(
            hits[0].score,
            100 + PIN_BOOST,
            "无历史时固定项获得 PIN_BOOST"
        );
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
            "固定+历史叠加后得分 {} 超过上限",
            hits[0].score
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
        apply_boosts(
            &mut hits,
            HashMap::new(),
            HashMap::new(),
            "q",
            1,
            &HashSet::new(),
        );
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
        apply_boosts(
            &mut hits,
            usage,
            pairs,
            "ter",
            1_700_086_400,
            &HashSet::new(),
        );
        assert_eq!(
            hits[0].item.id, "strong",
            "更高基础相关性的 word-prefix 不得被弱 substring+历史压过"
        );
    }

    #[test]
    fn query_pair_beats_global_usage_within_open_band() {
        use crate::search::ranker::SCORE_WORD_PREFIX;
        // 同基础分、无 Recency：A 仅全局高频，B 有本 Query 配对
        let mut popular = hit("popular");
        popular.score = SCORE_WORD_PREFIX;
        let mut chosen = hit("chosen");
        chosen.score = SCORE_WORD_PREFIX;
        let mut usage = HashMap::new();
        usage.insert(
            "popular".to_string(),
            UsageStats {
                launch_count: 80,
                last_used_at: 0,
            },
        );
        let mut pairs = HashMap::new();
        pairs.insert(
            "chosen".to_string(),
            QueryPairStats {
                count: 8,
                last_used_at: 0,
            },
        );
        let mut hits = vec![popular, chosen];
        apply_boosts(
            &mut hits,
            usage,
            pairs,
            "ter",
            1_700_086_400,
            &HashSet::new(),
        );
        assert_eq!(
            hits[0].item.id, "chosen",
            "本 Query 配对应压过仅全局 Usage 的同层候选"
        );
    }

    #[test]
    fn one_pair_selection_is_limited_and_repeats_grow_log() {
        let usage = UsageStats::default();
        let one = QueryPairStats {
            count: 1,
            last_used_at: 0,
        };
        let many = QueryPairStats {
            count: 30,
            last_used_at: 0,
        };
        let b1 = history_boost("q", &usage, &one, 0);
        let b30 = history_boost("q", &usage, &many, 0);
        assert!(b1 > 0, "一次选择应有有限倾向");
        assert!(b1 <= 40, "一次选择加分应有限，实际 {b1}");
        assert!(b30 > b1, "重复选择应增强");
        assert!(b30 <= HISTORY_BOOST_MAX);
        // 对数趋稳：30 次相对 10 次增幅有限
        let ten = QueryPairStats {
            count: 10,
            last_used_at: 0,
        };
        let b10 = history_boost("q", &usage, &ten, 0);
        assert!(
            b30 - b10 <= 25,
            "重复选择增幅应递减：10→{b10} 30→{b30}"
        );
    }

    #[test]
    fn name_exact_not_stolen_by_popular_open_candidate() {
        use crate::search::ranker::{SCORE_NAME_EXACT, SCORE_PREFIX};
        let mut exact = hit("exact-app");
        exact.score = SCORE_NAME_EXACT;
        let mut popular = hit("popular");
        popular.score = SCORE_PREFIX;
        let mut usage = HashMap::new();
        usage.insert(
            "popular".to_string(),
            UsageStats {
                launch_count: 200,
                last_used_at: 1_700_000_000,
            },
        );
        let mut pairs = HashMap::new();
        pairs.insert(
            "popular".to_string(),
            QueryPairStats {
                count: 50,
                last_used_at: 1_700_000_000,
            },
        );
        let mut hits = vec![popular, exact];
        apply_boosts(
            &mut hits,
            usage,
            pairs,
            "exact-app",
            1_700_086_400,
            &HashSet::new(),
        );
        assert_eq!(
            hits[0].item.id, "exact-app",
            "完整名称精确匹配不得被历史常用软件抢第一"
        );
    }

    #[test]
    fn empty_personalization_matches_base_order() {
        use crate::search::ranker::{SCORE_PREFIX, SCORE_WORD_PREFIX};
        let mut a = hit("a");
        a.score = SCORE_WORD_PREFIX;
        let mut b = hit("b");
        b.score = SCORE_PREFIX;
        let mut hits = vec![a, b];
        apply_personalization(&mut hits, &Personalization::default());
        assert_eq!(hits[0].item.id, "b", "无历史时按基础分");
        assert_eq!(hits[1].item.id, "a");
        assert!(!hits[0].matched_by.contains("history"));
    }

    #[test]
    fn pin_and_history_take_max_not_sum() {
        let mut hits = vec![hit("p")];
        let mut usage = HashMap::new();
        usage.insert(
            "p".to_string(),
            UsageStats {
                launch_count: 30,
                last_used_at: 1_700_000_000,
            },
        );
        let mut pairs = HashMap::new();
        pairs.insert(
            "p".to_string(),
            QueryPairStats {
                count: 20,
                last_used_at: 1_700_000_000,
            },
        );
        let pinned: HashSet<String> = ["p".to_string()].into_iter().collect();
        apply_boosts(
            &mut hits,
            usage,
            pairs,
            "p",
            1_700_086_400,
            &pinned,
        );
        let h = history_boost(
            "p",
            &UsageStats {
                launch_count: 30,
                last_used_at: 1_700_000_000,
            },
            &QueryPairStats {
                count: 20,
                last_used_at: 1_700_000_000,
            },
            1_700_086_400,
        );
        let expected = 100 + PIN_BOOST.max(h);
        assert_eq!(hits[0].score, expected, "Pin 与历史取较大者，不叠加");
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
        apply_boosts(
            &mut hits,
            usage,
            pairs,
            "ter",
            1_700_086_400,
            &HashSet::new(),
        );
        assert_eq!(
            hits[0].item.id, "xterminal",
            "同层内历史应把更常用项提到前面"
        );
    }

    #[test]
    fn quality_tier_orders_exact_before_fuzzy() {
        use crate::search::ranker::{
            SCORE_FUZZY_MAX, SCORE_NAME_EXACT, SCORE_PREFIX, SCORE_SUBSTRING,
        };
        assert!(quality_tier(SCORE_NAME_EXACT) < quality_tier(SCORE_PREFIX));
        assert!(quality_tier(SCORE_PREFIX) < quality_tier(SCORE_SUBSTRING));
        assert!(quality_tier(SCORE_SUBSTRING) < quality_tier(SCORE_FUZZY_MAX / 2));
    }

    #[test]
    fn equal_history_scores_have_a_stable_order() {
        let mut forward = vec![hit("b"), hit("a")];
        let mut reverse = vec![hit("a"), hit("b")];
        for hits in [&mut forward, &mut reverse] {
            apply_boosts(
                hits,
                HashMap::new(),
                HashMap::new(),
                "q",
                1,
                &HashSet::new(),
            );
        }

        let ids = |hits: &[SearchResult]| {
            hits.iter()
                .map(|hit| hit.item.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&forward), ids(&reverse));
        assert_eq!(ids(&forward), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn demote_pushes_equal_quality_peer_down_and_is_restorable() {
        let mut prefs = Personalization::default();
        prefs.query_norm = "code".into();
        prefs.demoted.insert("b".into());
        let mut hits = vec![hit("a"), hit("b")];
        apply_personalization(&mut hits, &prefs);
        assert_eq!(hits[0].item.id, "a");
        assert!(hits[1].matched_by.contains("+demote"));
        assert!(
            hits[1].score < hits[0].score,
            "降权后应低于同分未降权 peer"
        );

        prefs.demoted.clear();
        let mut hits2 = vec![hit("a"), hit("b")];
        apply_personalization(&mut hits2, &prefs);
        assert_eq!(hits2[0].score, hits2[1].score, "恢复后同分回默认稳定序");
    }

    #[test]
    fn demote_survives_max_history_boost() {
        let mut prefs = Personalization::default();
        prefs.query_norm = "code".into();
        prefs.demoted.insert("b".into());
        prefs.usage.insert(
            "b".into(),
            UsageStats {
                launch_count: 99,
                last_used_at: prefs.now,
            },
        );
        prefs.now = 1_700_086_400;
        let mut hits = vec![hit("a"), hit("b")];
        apply_personalization(&mut hits, &prefs);
        assert_eq!(
            hits[0].item.id, "a",
            "历史正向偏好不得轻易抵消降权: {:?}",
            hits.iter().map(|h| (&h.item.id, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pin_and_demote_tags_can_coexist() {
        let mut prefs = Personalization::default();
        prefs.pinned.insert("a".into());
        prefs.demoted.insert("a".into());
        let mut hits = vec![hit("a")];
        apply_personalization(&mut hits, &prefs);
        assert!(hits[0].matched_by.contains("+pin"));
        assert!(hits[0].matched_by.contains("+demote"));
    }

    #[test]
    fn demote_does_not_hide_protected_name_exact() {
        let mut exact = hit("exact");
        exact.score = 1000;
        exact.quality_tier = 1;
        let mut weak = hit("weak");
        weak.score = 500;
        weak.quality_tier = 4;
        let mut prefs = Personalization::default();
        prefs.demoted.insert("exact".into());
        let mut hits = vec![exact, weak];
        apply_personalization(&mut hits, &prefs);
        assert_eq!(
            hits[0].item.id, "exact",
            "保护层 Name Exact 不得被降权挤出前排"
        );
    }

    #[test]
    fn preference_adjust_is_single_source_for_tags_and_boost() {
        let mut prefs = Personalization::default();
        prefs.pinned.insert("a".into());
        prefs.demoted.insert("b".into());
        prefs.usage.insert(
            "c".into(),
            UsageStats {
                launch_count: 8,
                last_used_at: prefs.now,
            },
        );
        prefs.now = 1_700_086_400;
        let adj_pin = preference_adjust(&prefs, "a", 4);
        assert!(adj_pin.pinned && adj_pin.boost >= PIN_BOOST);
        assert!(apply_preference_tags("prefix", &adj_pin).contains("+pin"));
        let adj_dem = preference_adjust(&prefs, "b", 4);
        assert!(adj_dem.demoted && adj_dem.boost < 0);
        assert!(apply_preference_tags("prefix", &adj_dem).contains("+demote"));
        let adj_hist = preference_adjust(&prefs, "c", 4);
        assert!(adj_hist.history && adj_hist.boost > 0);
    }

    #[test]
    fn cache_path_and_ranked_path_produce_same_order() {
        use crate::search::{search_with_personalization, RetrievalIndex};
        use crate::model::AppItem;

        fn app(id: &str, name: &str) -> AppItem {
            let mut it = AppItem::scanned(
                id.into(),
                name.into(),
                format!(r"C:\{id}.exe"),
                None,
                None,
                "t",
            );
            it.attach_search_fields();
            it
        }
        let index = RetrievalIndex::build(
            &[app("a", "Code Alpha"), app("b", "Code Beta"), app("c", "Code Gamma")],
            &[],
        );
        let mut prefs = Personalization::default();
        prefs.query_norm = "code".into();
        prefs.demoted.insert("b".into());
        prefs.usage.insert(
            "c".into(),
            UsageStats {
                launch_count: 20,
                last_used_at: prefs.now,
            },
        );
        prefs.now = 1_700_086_400;

        // 主路径：RankedHit 个性化
        let ranked = search_with_personalization(&index, "code", &[], Some(&prefs), 10);
        // 缓存路径：先无个性化基础结果，再 personalize_base_hits
        let base = search_with_personalization(&index, "code", &[], None, 10);
        let replayed = crate::search::service::personalize_base_hits(base, Some(&prefs));
        let ranked_ids: Vec<_> = ranked.iter().map(|h| h.item.id.as_str()).collect();
        let replay_ids: Vec<_> = replayed.iter().map(|h| h.item.id.as_str()).collect();
        assert_eq!(
            ranked_ids, replay_ids,
            "缓存命中与主路径最终顺序必须一致"
        );
    }
}
