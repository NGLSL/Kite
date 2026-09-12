//! 搜索管线：规范化 → 多路召回 → 统一评分 → Top N。

mod alias;
mod fuzzy;
mod matcher;
mod normalizer;
mod pinyin;
mod ranker;
pub mod url;

use crate::model::{AppItem, SearchResult};

/// 默认返回条数。
pub const TOP_N: usize = 10;

/// 索引用名称规范化（供 AppItem 预计算）。
pub fn normalize_for_index(name: &str) -> String {
    normalizer::normalize_name(name)
}

/// 索引用拼音预计算：返回 (全拼无空格, 首字母)。
pub fn pinyin_of(text: &str) -> (String, String) {
    pinyin::precompute(text)
}

/// 入口：空 Query 给默认列表，否则多路召回 + 排序。
/// `user_alias_targets`：用户 Alias 命中的应用名（小写）。
pub fn search(apps: &[AppItem], query: &str, user_alias_targets: &[String]) -> Vec<SearchResult> {
    let q = normalizer::normalize_query(query);
    if q.is_empty() {
        // 无历史时退回索引顺序；有历史请调用方走 order_by_recent
        return apps
            .iter()
            .take(TOP_N)
            .map(|item| SearchResult {
                item: item.clone(),
                score: 0,
                matched_by: "default".into(),
            })
            .collect();
    }

    let hits = matcher::collect_candidates(apps, &q, user_alias_targets);
    ranker::rank_and_truncate(hits, TOP_N)
}

/// 空 Query 默认列表：最近使用优先，不足再按索引顺序补满。
pub fn order_by_recent(apps: &[AppItem], recent_ids: &[String], top_n: usize) -> Vec<SearchResult> {
    let mut hits: Vec<SearchResult> = Vec::with_capacity(top_n.min(apps.len()));
    for id in recent_ids {
        if hits.len() >= top_n {
            break;
        }
        if hits.iter().any(|h| h.item.id == *id) {
            continue;
        }
        if let Some(item) = apps.iter().find(|a| a.id == *id) {
            hits.push(SearchResult {
                item: item.clone(),
                score: 1,
                matched_by: "recent".into(),
            });
        }
    }
    for item in apps {
        if hits.len() >= top_n {
            break;
        }
        if hits.iter().any(|h| h.item.id == item.id) {
            continue;
        }
        hits.push(SearchResult {
            item: item.clone(),
            score: 0,
            matched_by: "default".into(),
        });
    }
    hits
}

/// 历史加分后重新排序截断（commands 在改分后调用）。
pub fn rerank(hits: Vec<SearchResult>, top_n: usize) -> Vec<SearchResult> {
    ranker::rank_and_truncate(hits, top_n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> AppItem {
        let mut it = AppItem::scanned(
            name.to_string(),
            name.to_string(),
            format!("C:\\fake\\{name}.exe"),
            None,
            None,
            "test",
        );
        it.attach_search_fields();
        it
    }

    /// 回归用例：Query → 期望排第一的应用名。修 bug 时往这里加。
    fn assert_top(query: &str, expected: &str) {
        let apps = vec![
            item("Google Chrome"),
            item("Chrome Remote Desktop"),
            item("Visual Studio Code"),
            item("Visual Studio"),
            item("微信"),
            item("微信开发者工具"),
            item("企业微信"),
            item("IntelliJ IDEA"),
            item("Notepad"),
        ];
        let hits = search(&apps, query, &[]);
        assert!(
            !hits.is_empty(),
            "no hits for {query}"
        );
        assert_eq!(
            hits[0].item.name, expected,
            "query={query} top={} expected={expected}",
            hits[0].item.name
        );
    }

    #[test]
    fn exact_beats_prefix() {
        assert_top("chrome", "Google Chrome");
    }

    #[test]
    fn prefix_matches() {
        // 同为 prefix 时短名优先（Phase 2 无历史；想要 VS Code 用 vsc/vscode）
        assert_top("vis", "Visual Studio");
    }

    #[test]
    fn chinese_exact() {
        assert_top("微信", "微信");
    }

    #[test]
    fn alias_vscode() {
        assert_top("vsc", "Visual Studio Code");
        assert_top("vscode", "Visual Studio Code");
    }

    #[test]
    fn alias_wechat() {
        assert_top("wx", "微信");
        assert_top("weixin", "微信");
    }

    #[test]
    fn alias_idea() {
        assert_top("idea", "IntelliJ IDEA");
    }

    #[test]
    fn pinyin_full() {
        assert_top("weixin", "微信");
    }

    #[test]
    fn pinyin_initials() {
        assert_top("wx", "微信");
        assert_top("wxkf", "微信开发者工具");
    }

    #[test]
    fn substring() {
        assert_top("studio", "Visual Studio");
    }

    #[test]
    fn fuzzy_typo() {
        assert_top("chorme", "Google Chrome");
        assert_top("crome", "Google Chrome");
    }

    #[test]
    fn query_history_style_protection_via_alias() {
        // chrome 本身应精确命中 Google Chrome，而不是 Remote Desktop
        assert_top("chrome", "Google Chrome");
    }

    #[test]
    fn history_boost_cannot_beat_exact_gap() {
        // Name Exact(1000) + 历史上限(160) 仍应高于 Prefix(800) + 同样历史上限
        let exact = crate::search::ranker::SCORE_NAME_EXACT
            + crate::history::HISTORY_BOOST_MAX;
        let prefix = crate::search::ranker::SCORE_PREFIX
            + crate::history::HISTORY_BOOST_MAX;
        assert!(exact > prefix);
    }

    #[test]
    fn empty_query_recent_first() {
        let apps = vec![item("A"), item("B"), item("C")];
        let recent = vec![apps[2].id.clone(), apps[0].id.clone()];
        let hits = order_by_recent(&apps, &recent, 3);
        assert_eq!(hits[0].item.name, "C");
        assert_eq!(hits[0].matched_by, "recent");
        assert_eq!(hits[1].item.name, "A");
        assert_eq!(hits[2].item.name, "B");
        assert_eq!(hits[2].matched_by, "default");
    }

    #[test]
    fn empty_query_recent_missing_id_skipped() {
        let apps = vec![item("A"), item("B")];
        let recent = vec!["ghost".into(), apps[1].id.clone()];
        let hits = order_by_recent(&apps, &recent, 2);
        assert_eq!(hits[0].item.name, "B");
        assert_eq!(hits[1].item.name, "A");
    }

    #[test]
    fn empty_query_no_duplicate() {
        let apps = vec![item("A"), item("B")];
        let recent = vec![apps[0].id.clone(), apps[0].id.clone()];
        let hits = order_by_recent(&apps, &recent, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].item.name, "A");
        assert_eq!(hits[1].item.name, "B");
    }
}
