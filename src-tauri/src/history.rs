//! 历史加权：把 Usage / Recency / Query History 映射为排序加分。
//! 原则（PRD §41–42）：
//! - MatchScore 仍是主信号
//! - 历史总加分有上限，不得让 Prefix 压过 Name Exact
//! - 不做纯 LRU

use crate::storage::{QueryPairStats, UsageStats};

/// 历史总加分上限。须 < (SCORE_NAME_EXACT - SCORE_PREFIX) = 200。
pub const HISTORY_BOOST_MAX: i32 = 160;

const QUERY_PAIR_CAP: i32 = 90;
const FREQUENCY_CAP: i32 = 50;
const RECENCY_CAP: i32 = 40;

/// 计算历史加分（已 clamp 到 HISTORY_BOOST_MAX）。
pub fn history_boost(query: &str, usage: &UsageStats, pair: &QueryPairStats, now: i64) -> i32 {
    let q = query_pair_score(pair);
    let f = frequency_score(usage.launch_count);
    let r = recency_score(usage.last_used_at, now);
    (q + f + r).min(HISTORY_BOOST_MAX)
}

/// Query→App 配对越稳越高；对数增长，封顶。
fn query_pair_score(pair: &QueryPairStats) -> i32 {
    if pair.count <= 0 {
        return 0;
    }
    // 1→20, 3→38, 10→60, 30→~85
    let s = 20.0 + 15.0 * (pair.count as f64).ln();
    s.round() as i32
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
}
