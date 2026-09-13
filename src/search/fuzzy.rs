//! 模糊匹配：Damerau-Levenshtein，处理相邻交换（chorme → chrome）。

use crate::search::ranker::{fuzzy_score, SCORE_FUZZY_MAX};

/// 按 Query 长度限制最大编辑距离（PRD §31）。
pub fn max_distance(q_len: usize) -> usize {
    if q_len <= 2 {
        0 // 短 Query 禁止大范围 fuzzy
    } else if q_len <= 5 {
        1
    } else {
        2
    }
}

/// 返回 (score, distance)；超阈值 None。
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<(i32, usize)> {
    if query.is_empty() || candidate.is_empty() {
        return None;
    }
    let max_d = max_distance(query.len());
    if max_d == 0 {
        return None;
    }
    let d = damerau_levenshtein(query, candidate);
    if d <= max_d && d > 0 {
        let s = fuzzy_score(d).clamp(0, SCORE_FUZZY_MAX);
        Some((s, d))
    } else {
        None
    }
}

/// 标准 Damerau-Levenshtein（允许相邻交换）。
pub fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        d[i][0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }

    for i in 1..=n {
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_swap() {
        assert_eq!(damerau_levenshtein("chorme", "chrome"), 1);
        assert_eq!(damerau_levenshtein("crome", "chrome"), 1);
    }

    #[test]
    fn short_query_no_fuzzy() {
        assert!(fuzzy_match("ch", "chrome").is_none());
    }
}
