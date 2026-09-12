//! 搜索管线：规范化 Query、匹配、排序、截断。

mod matcher;

use crate::model::{AppItem, SearchResult};

pub const TOP_N: usize = 10;

/// Phase 1 分数。集中定义，禁止散落到各 matcher。
pub const SCORE_EXACT: i32 = 1000;
pub const SCORE_PREFIX: i32 = 800;

pub fn search(apps: &[AppItem], query: &str) -> Vec<SearchResult> {
    let q = matcher::normalize_query(query);
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

    let mut hits: Vec<SearchResult> = apps
        .iter()
        .filter_map(|item| {
            let (score, matched_by) = matcher::match_item(item, &q)?;
            Some(SearchResult {
                item: item.clone(),
                score,
                matched_by: matched_by.to_string(),
            })
        })
        .collect();

    // 分数优先；同分短名优先；再按名称字典序保证稳定。
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.item.name.len().cmp(&b.item.name.len()))
            .then_with(|| a.item.name.to_lowercase().cmp(&b.item.name.to_lowercase()))
    });
    hits.truncate(TOP_N);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> AppItem {
        AppItem {
            id: name.to_string(),
            name: name.to_string(),
            display_name: name.to_string(),
            target: format!("C:\\fake\\{name}.exe"),
            args: None,
            working_dir: None,
            icon: None,
            source: "test".into(),
        }
    }

    #[test]
    fn exact_beats_prefix() {
        let apps = vec![item("Google Chrome"), item("Chrome Remote Desktop")];
        let hits = search(&apps, "chrome");
        assert_eq!(hits[0].item.name, "Google Chrome");
    }

    #[test]
    fn prefix_matches() {
        let apps = vec![item("Visual Studio Code"), item("Notepad")];
        let hits = search(&apps, "vis");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_by, "prefix");
    }
}
