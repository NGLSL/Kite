//! 搜索管线：规范化 → 多路召回 → 统一评分 → Top N。

mod alias;
#[cfg(test)]
mod bench;
#[cfg(test)]
mod eval;
mod fuzzy;
mod matcher;
mod normalizer;
mod pinyin;
pub mod ranker;
pub mod url;

pub use matcher::UserTarget;

use crate::model::{AppItem, SearchResult};

/// 默认返回条数（首页）。
pub const TOP_N: usize = 10;

/// 排序结果缓存/返回上限；滚动加载最多翻到这里。
pub const MAX_RESULTS: usize = 200;

/// 索引用名称规范化（供 AppItem 预计算）。
pub fn normalize_for_index(name: &str) -> String {
    normalizer::normalize_name(name)
}

/// 索引用拼音预计算：返回 (全拼无空格, 首字母)。
pub fn pinyin_of(text: &str) -> (String, String) {
    pinyin::precompute(text)
}

/// 入口：空 Query 给默认列表，否则多路召回 + 排序。
/// `user_alias_targets`：用户 Alias 目标（稳定 id / 名称）。
/// `max_results`：返回条数上限（滚动加载时调用方逐步放大；截断在 IPC 边界做）。
pub fn search(
    apps: &[AppItem],
    query: &str,
    user_alias_targets: &[UserTarget],
    max_results: usize,
) -> Vec<SearchResult> {
    let q = normalizer::normalize_query(query);
    if q.is_empty() {
        // 无历史时退回索引顺序；有历史请调用方走 order_by_recent
        return apps
            .iter()
            .take(max_results)
            .map(|item| SearchResult {
                item: item.clone(),
                score: 0,
                matched_by: "default".into(),
            })
            .collect();
    }

    let mut hits = matcher::collect_candidates(apps, &q, user_alias_targets);
    ranker::prefer_friendly_install_entries(&mut hits);
    ranker::rank_and_truncate(hits, max_results)
}

/// 空 Query 默认列表：固定项优先，其次最近使用，不足再按索引顺序补满。
pub fn order_by_recent(
    apps: &[AppItem],
    recent_ids: &[String],
    pinned_ids: &[String],
    top_n: usize,
) -> Vec<SearchResult> {
    let mut hits: Vec<SearchResult> = Vec::with_capacity(top_n.min(apps.len()));
    let push_hit = |id: &str, score: i32, matched_by: &'static str, hits: &mut Vec<SearchResult>| -> bool {        if hits.iter().any(|h| h.item.id == id) {
            return false;
        }
        if let Some(item) = apps.iter().find(|a| a.id == id) {
            hits.push(SearchResult {
                item: item.clone(),
                score,
                matched_by: matched_by.into(),
            });
            return true;
        }
        false
    };
    for id in pinned_ids {
        if hits.len() >= top_n {
            break;
        }
        push_hit(id, 2, "pinned", &mut hits);
    }
    for id in recent_ids {
        if hits.len() >= top_n {
            break;
        }
        push_hit(id, 1, "recent", &mut hits);
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

/// Alias 目标选择器用：按名称/拼音从索引挑候选，Top N。
/// 轻量实现（前缀 > 包含，短名优先），不走完整评分管线。
pub fn name_candidates(apps: &[AppItem], query: &str, top_n: usize) -> Vec<SearchResult> {
    fn norm<'a>(precomputed: &'a str, raw: &'a str) -> std::borrow::Cow<'a, str> {
        if precomputed.is_empty() {
            std::borrow::Cow::Owned(normalizer::normalize_name(raw))
        } else {
            std::borrow::Cow::Borrowed(precomputed)
        }
    }

    let q = normalizer::normalize_query(query);
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<SearchResult> = Vec::new();
    for item in apps {
        let name = norm(&item.normalized_name, &item.name);
        let display = norm(&item.normalized_display, &item.display_name);
        let mut score = 0i32;
        if name == q || display == q {
            score = 4;
        } else if name.starts_with(&*q) || display.starts_with(&*q) {
            score = 3;
        } else if !item.pinyin.is_empty() && item.pinyin.starts_with(&*q) {
            score = 2;
        } else if name.contains(&*q) || display.contains(&*q) {
            score = 1;
        } else if !item.pinyin_initials.is_empty() && item.pinyin_initials.starts_with(&*q) {
            score = 1;
        }
        if score > 0 {
            hits.push(SearchResult {
                item: item.clone(),
                score,
                matched_by: "candidate".into(),
            });
        }
    }
    ranker::rank_and_truncate(hits, top_n)
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

    fn sourced_item(name: &str, target: &str, source: &str) -> AppItem {
        let mut it = AppItem::scanned(
            format!("{source}:{target}"),
            name.into(),
            target.into(),
            None,
            None,
            source,
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
            item("网易云音乐"),
            item("Steam"),
            item("计算器"),
            item("Windows Terminal"),
        ];
        let hits = search(&apps, query, &[], TOP_N);
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
    fn alias_new_builtin_spots() {
        // 各领域抽查：扩充内置 Alias 后仍要排第一
        assert_top("wyy", "网易云音乐");
        assert_top("steam", "Steam");
        assert_top("calc", "计算器");
        assert_top("wt", "Windows Terminal");
    }

    #[test]
    fn alias_vs_still_finds_vscode_first_available() {
        // "vs" 片段覆盖 VS 与 VS Code；两者都在时短名 Visual Studio 在前
        assert_top("vs", "Visual Studio");
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
    fn friendly_shortcut_ranks_above_raw_app_path_in_same_installation() {
        let raw = sourced_item(
            "wps",
            r"D:\Program Files\WPS Office\12.1.0\office6\wps.exe",
            "app-paths",
        );
        let shortcut = sourced_item(
            "WPS Office",
            r"D:\Program Files\WPS Office\ksolaunch.exe",
            "start-menu",
        );

        let hits = search(&[raw, shortcut], "wps", &[], TOP_N);
        assert_eq!(hits.len(), 2, "原始入口应继续可搜索");
        assert_eq!(hits[0].item.name, "WPS Office");
        assert_eq!(hits[1].item.name, "wps");
        assert!(hits[0].score > hits[1].score);
        assert_eq!(rerank(hits.clone(), TOP_N)[1].score, hits[1].score);
    }

    #[test]
    fn source_preference_applies_to_other_apps_and_preserves_unrelated_exact_match() {
        let raw = sourced_item("aurora", r"D:\Apps\Aurora\bin\aurora.exe", "app-paths");
        let friendly = sourced_item("Aurora Studio", r"d:\apps\aurora\launcher.exe", "desktop");
        let hits = search(&[raw.clone(), friendly], "aurora", &[], TOP_N);
        assert_eq!(hits[0].item.name, "Aurora Studio");
        assert_eq!(hits[1].item.name, "aurora");

        let unrelated = sourced_item("Aurora Remote", r"D:\Apps\Remote\launcher.exe", "desktop");
        let hits = search(&[raw, unrelated], "aurora", &[], TOP_N);
        assert_eq!(hits[0].item.name, "aurora", "无同安装目录的友好入口时保留精确匹配");
    }

    #[test]
    fn empty_query_recent_first() {
        let apps = vec![item("A"), item("B"), item("C")];
        let recent = vec![apps[2].id.clone(), apps[0].id.clone()];
        let hits = order_by_recent(&apps, &recent, &[], 3);
        assert_eq!(hits[0].item.name, "C");
        assert_eq!(hits[0].matched_by, "recent");
        assert_eq!(hits[1].item.name, "A");
        assert_eq!(hits[2].item.name, "B");
        assert_eq!(hits[2].matched_by, "default");
    }

    #[test]
    fn empty_query_pinned_before_recent() {
        let apps = vec![item("A"), item("B"), item("C")];
        let recent = vec![apps[2].id.clone()];
        let pinned = vec![apps[0].id.clone()];
        let hits = order_by_recent(&apps, &recent, &pinned, 3);
        assert_eq!(hits[0].item.name, "A");
        assert_eq!(hits[0].matched_by, "pinned");
        assert_eq!(hits[1].item.name, "C");
        assert_eq!(hits[1].matched_by, "recent");
        assert_eq!(hits[2].item.name, "B");
    }

    #[test]
    fn pinned_order_follows_pin_time_desc() {
        let apps = vec![item("A"), item("B"), item("C")];
        let hits = order_by_recent(&apps, &[], &[apps[2].id.clone(), apps[0].id.clone()], 3);
        assert_eq!(hits[0].item.name, "C");
        assert_eq!(hits[1].item.name, "A");
    }

    #[test]
    fn empty_query_recent_missing_id_skipped() {
        let apps = vec![item("A"), item("B")];
        let recent = vec!["ghost".into(), apps[1].id.clone()];
        let hits = order_by_recent(&apps, &recent, &[], 2);
        assert_eq!(hits[0].item.name, "B");
        assert_eq!(hits[1].item.name, "A");
    }

    #[test]
    fn empty_query_pinned_missing_id_skipped() {
        let apps = vec![item("A"), item("B")];
        let hits = order_by_recent(&apps, &[], &["ghost".into(), apps[0].id.clone()], 2);
        assert_eq!(hits[0].item.name, "A");
        assert_eq!(hits[1].item.name, "B");
    }

    #[test]
    fn empty_query_no_duplicate() {
        let apps = vec![item("A"), item("B")];
        let recent = vec![apps[0].id.clone(), apps[0].id.clone()];
        let hits = order_by_recent(&apps, &recent, &[apps[0].id.clone()], 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].item.name, "A");
        assert_eq!(hits[1].item.name, "B");
    }

    #[test]
    fn name_candidates_rank_prefix_over_contains() {
        let apps: Vec<_> = ["Google Chrome", "Chrome Remote Desktop", "My Chrome Box"]
            .iter()
            .map(|n| item(n))
            .collect();
        let hits = name_candidates(&apps, "chro", 8);
        // 只有 "Chrome Remote Desktop" 是前缀；另两条靠包含命中，同分短名在前
        assert_eq!(hits[0].item.name, "Chrome Remote Desktop");
        assert_eq!(hits[1].item.name, "Google Chrome");
        assert_eq!(hits[2].item.name, "My Chrome Box");
        assert!(hits.iter().all(|h| h.matched_by == "candidate"));
    }

    #[test]
    fn name_candidates_supports_pinyin() {
        let apps = vec![item("微信")];
        assert_eq!(name_candidates(&apps, "weixin", 8).len(), 1);
        assert_eq!(name_candidates(&apps, "wx", 8).len(), 1);
        assert!(name_candidates(&apps, "zzz", 8).is_empty());
    }

    #[test]
    fn name_candidates_empty_query_and_limit() {
        let apps: Vec<_> = (0..10).map(|i| item(&format!("App{i:02}"))).collect();
        assert!(name_candidates(&apps, "  ", 8).is_empty(), "空 Query 无候选");
        assert_eq!(name_candidates(&apps, "app", 3).len(), 3, "只返回 Top N");
    }

    #[test]
    fn max_results_caps_output() {
        let apps: Vec<_> = (0..20).map(|i| item(&format!("App{i:02}"))).collect();
        assert_eq!(search(&apps, "app", &[], 3).len(), 3);
        assert_eq!(search(&apps, "app", &[], 100).len(), 20);
    }

    #[test]
    fn pages_are_consistent_prefixes() {
        // 滚动加载契约：前 K 条与一次取更多时的前 K 条完全一致
        let apps: Vec<_> = (0..15).map(|i| item(&format!("App{i:02}"))).collect();
        let full = search(&apps, "app", &[], 15);
        let first = search(&apps, "app", &[], 5);
        assert_eq!(full.len(), 15);
        assert_eq!(first.len(), 5);
        for (i, hit) in first.iter().enumerate() {
            assert_eq!(hit.item.id, full[i].item.id, "第 {i} 页不一致");
        }
    }

    #[test]
    fn pinyin_initials_match_word_inside_name() {
        // kz = 「控制」首字母：既应命中控制面板（前缀），也应命中名称中间/末尾含「控制」的应用
        let apps = vec![
            item("控制面板"),
            item("向日葵远程控制"),
            item("键盘控制鼠标设置"),
            item("反馈中心"),
            item("记事本"),
        ];
        let hits = search(&apps, "kz", &[], TOP_N);
        let names: Vec<_> = hits.iter().map(|h| h.item.name.as_str()).collect();
        assert!(names.contains(&"控制面板"), "前缀命中: {names:?}");
        assert!(names.contains(&"向日葵远程控制"), "末尾控制: {names:?}");
        assert!(names.contains(&"键盘控制鼠标设置"), "中间控制: {names:?}");
        assert!(
            !names.contains(&"记事本"),
            "无关应用不应被 kz 召回: {names:?}"
        );
    }

    #[test]
    fn exp_finds_file_explorer_via_word_and_name() {
        let apps = vec![item("File Explorer"), item("IEXPLORE")];
        let hits = search(&apps, "exp", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "File Explorer"),
            "exp 应命中 File Explorer（explorer 词前缀）: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }
}
