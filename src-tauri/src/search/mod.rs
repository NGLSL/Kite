//! 搜索管线：规范化 → 多路召回 → 统一评分 → Top N。

mod alias;
mod fuzzy;
mod matcher;
mod normalizer;
mod pinyin;
mod ranker;

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
pub fn search(apps: &[AppItem], query: &str) -> Vec<SearchResult> {
    let q = normalizer::normalize_query(query);
    if q.is_empty() {
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

    let hits = matcher::collect_candidates(apps, &q);
    ranker::rank_and_truncate(hits, TOP_N)
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
        let hits = search(&apps, query);
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
}
