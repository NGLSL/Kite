//! 搜索公共入口行为测试（统一搜索缝）。

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
        assert!(!hits.is_empty(), "no hits for {query}");
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
        let exact = crate::search::ranker::SCORE_NAME_EXACT + crate::history::HISTORY_BOOST_MAX;
        let prefix = crate::search::ranker::SCORE_PREFIX + crate::history::HISTORY_BOOST_MAX;
        assert!(exact > prefix);
    }

    #[test]
    fn friendly_shortcut_hides_raw_app_path_in_same_installation() {
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
        assert_eq!(
            hits.iter().map(|h| h.item.name.as_str()).collect::<Vec<_>>(),
            vec!["WPS Office"],
            "同安装目录已有友好入口时，app-paths 裸 exe 不应并排出现: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn same_display_name_from_start_menu_and_apps_folder_collapses() {
        let lnk = sourced_item(
            "微信",
            r"C:\Program Files (x86)\Tencent\WeChat\WeChat.exe",
            "start-menu",
        );
        let shell = sourced_item(
            "微信",
            r"shell:AppsFolder\TencentWeChat_abc!App",
            "apps-folder",
        );
        let hits = search(&[lnk, shell], "微信", &[], TOP_N);
        assert_eq!(
            hits.iter().map(|h| h.item.name.as_str()).collect::<Vec<_>>(),
            vec!["微信"],
            "同名微信应只保留开始菜单入口: {:?}",
            hits.iter()
                .map(|h| (&h.item.source, &h.item.target))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[0].item.source, "start-menu");
    }

    #[test]
    fn same_name_start_menu_and_desktop_keep_single_friendly() {
        let menu = sourced_item("微信", r"C:\A\WeChat\WeChat.exe", "start-menu");
        let desk = sourced_item("微信", r"C:\B\Weixin\Weixin.exe", "desktop");
        let hits = search(&[menu, desk], "微信", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "同名两条友好入口也只留一条: {:?}",
            hits.iter().map(|h| &h.item.target).collect::<Vec<_>>()
        );
        assert_eq!(hits[0].item.source, "start-menu");
    }

    #[test]
    fn raw_app_path_remains_when_no_friendly_entry_in_results() {
        let raw = sourced_item(
            "wps",
            r"D:\Program Files\WPS Office\12.1.0\office6\wps.exe",
            "app-paths",
        );
        let hits = search(&[raw], "wps", &[], TOP_N);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.name, "wps");
    }

    #[test]
    fn source_preference_applies_to_other_apps_and_preserves_unrelated_exact_match() {
        let raw = sourced_item("aurora", r"D:\Apps\Aurora\bin\aurora.exe", "app-paths");
        let friendly = sourced_item("Aurora Studio", r"d:\apps\aurora\launcher.exe", "desktop");
        let hits = search(&[raw.clone(), friendly], "aurora", &[], TOP_N);
        assert_eq!(
            hits.iter().map(|h| h.item.name.as_str()).collect::<Vec<_>>(),
            vec!["Aurora Studio"],
            "同安装目录友好入口应隐藏 app-paths: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );

        let unrelated = sourced_item("Aurora Remote", r"D:\Apps\Remote\launcher.exe", "desktop");
        let hits = search(&[raw, unrelated], "aurora", &[], TOP_N);
        assert_eq!(
            hits[0].item.name, "aurora",
            "无同安装目录的友好入口时保留精确匹配"
        );
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
        assert!(
            name_candidates(&apps, "  ", 8).is_empty(),
            "空 Query 无候选"
        );
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
    fn kj_recalls_indexed_names_with_inner_pinyin_initials() {
        let apps = vec![item("开机启动设置"), item("存储空间"), item("记事本")];
        let hits = search(&apps, "kj", &[], TOP_N);
        let names: Vec<_> = hits.iter().map(|hit| hit.item.name.as_str()).collect();

        for expected in ["开机启动设置", "存储空间"] {
            assert!(
                names.contains(&expected),
                "已索引名称应被 kj 召回: {names:?}"
            );
        }
        assert!(!names.contains(&"记事本"), "无关项不应命中: {names:?}");
    }

    #[test]
    fn kj_recalls_existing_windows_startup_page() {
        let entries = crate::app::builtin::materialize_system_entries(None);
        let hits = search_with_system(&[], &entries, "kj", &[], TOP_N);
        assert!(
            hits.iter()
                .any(|hit| hit.item.target == "ms-settings:startupapps"),
            "Windows 启动设置入口应能用 kj 找到: {:?}",
            hits.iter().map(|hit| &hit.item.name).collect::<Vec<_>>()
        );
        let startup_pos = hits
            .iter()
            .position(|hit| hit.item.target == "ms-settings:startupapps")
            .unwrap();
        if let Some(storage_pos) = hits.iter().position(|hit| hit.item.name == "存储空间") {
            assert!(
                startup_pos < storage_pos,
                "开机启动入口应排在仅含内部拼音片段的存储空间之前: {:?}",
                hits.iter()
                    .map(|hit| (&hit.item.name, hit.score, &hit.matched_by))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn trusted_keyword_initials_beat_loose_name_pinyin() {
        let mut intended = item("启动");
        intended.search_keywords = vec!["开机启动".into()];
        let loose = item("存储空间");
        let hits = search(&[loose, intended], "kj", &[], TOP_N);
        assert_eq!(
            hits.first().map(|hit| hit.item.name.as_str()),
            Some("启动"),
            "可信词简拼前缀应排在宽松的名称拼音命中之前: {:?}",
            hits.iter()
                .map(|hit| (&hit.item.name, hit.score, &hit.matched_by))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[0].matched_by, "keyword-initials-prefix");
    }

    #[test]
    #[ignore = "Requires current Windows Settings search resources and zh-CN terms"]
    fn windows_settings_standard_terms_are_indexed_without_new_entries() {
        let entries = crate::app::builtin::materialize_system_entries(None);
        let enriched_pages = entries
            .iter()
            .filter(|entry| {
                entry.id.starts_with("winsettings:") && !entry.search_context.is_empty()
            })
            .count();
        assert!(enriched_pages >= 30, "Windows 标准词应批量补充现有设置页");
        let startup = entries
            .iter()
            .find(|entry| entry.target == "ms-settings:startupapps")
            .expect("existing startup settings page");
        assert!(
            startup.search_context.iter().any(|term| term == "启动任务"),
            "Windows search resources should enrich the existing page"
        );
        let hits = search_with_system(&[], &entries, "启动任务", &[], TOP_N);
        assert!(
            hits.iter()
                .any(|hit| hit.item.target == startup.target
                    && hit.matched_by.starts_with("context-")),
            "standard Windows term should recall the existing startup page: {:?}",
            hits.iter()
                .map(|hit| (&hit.item.name, &hit.matched_by))
                .collect::<Vec<_>>()
        );
        let initial_hits = search_with_system(&[], &entries, "qdrw", &[], TOP_N);
        assert!(
            initial_hits
                .iter()
                .any(|hit| hit.item.target == startup.target
                    && hit.matched_by.starts_with("context-")),
            "Windows 标准词应自动提供短简拼检索"
        );
    }

    #[test]
    fn system_context_recalls_pinyin_but_does_not_beat_exact_name() {
        let mut weak = item("某设置");
        weak.search_context = vec!["启动任务".into()];
        let exact = item("启动任务");
        let hits = search(&[weak, exact], "启动任务", &[], TOP_N);
        assert_eq!(hits[0].item.name, "启动任务");
        assert!(
            hits.iter()
                .any(|hit| hit.item.name == "某设置" && hit.matched_by.starts_with("context-")),
            "Windows 标准词应扩大召回，而不压过真实名称"
        );

        let mut context_only = item("某设置");
        context_only.search_context = vec!["启动任务".into()];
        let hits = search(&[context_only], "qdrw", &[], TOP_N);
        assert!(
            hits.iter()
                .any(|hit| hit.item.name == "某设置" && hit.matched_by.starts_with("context-")),
            "标准中文词应自动支持拼音简拼"
        );
    }

    #[test]
    fn clear_system_queries_keep_the_intended_entry_first() {
        let entries = crate::app::builtin::materialize_system_entries(None);
        for (query, expected_id) in [
            ("kj", "winsettings:ms-settings:startupapps"),
            ("蓝牙", "winsettings:ms-settings:bluetooth"),
            ("文件资源管理器", "system-tool:file-explorer"),
        ] {
            let hits = search_with_system(&[], &entries, query, &[], TOP_N);
            assert_eq!(
                hits.first().map(|hit| hit.item.id.as_str()),
                Some(expected_id),
                "明确系统查询 {query:?} 的前排错误: {:?}",
                hits.iter()
                    .map(|hit| (&hit.item.name, hit.score, &hit.matched_by))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn short_system_entry_strong_hits_match_unpruned_reference() {
        use std::collections::BTreeSet;

        let mut entries = crate::app::builtin::materialize_system_entries(None);
        entries.retain(|entry| {
            entry.id.starts_with("winsettings:") || entry.id.starts_with("system-tool:")
        });
        let index = RetrievalIndex::build(&[], &entries);
        let mut probes = BTreeSet::new();
        for doc in &index.docs {
            for field in [&doc.pinyin_initials, &doc.name] {
                let chars: Vec<char> = field.chars().collect();
                for pair in chars.windows(2) {
                    if !pair.iter().all(|ch| ch.is_alphanumeric()) {
                        continue;
                    }
                    let query = normalizer::normalize_query(&pair.iter().collect::<String>());
                    if query.chars().count() == 2 {
                        probes.insert(query);
                    }
                }
            }
            for keyword in &doc.keywords {
                let (_, initials) = pinyin_of(keyword);
                if initials.chars().count() >= 2 {
                    probes.insert(initials.chars().take(2).collect::<String>());
                }
            }
            for field in &doc.context_fields {
                let prefix = field.chars().take(2).collect::<String>();
                if prefix.chars().count() == 2 && prefix.chars().all(char::is_alphanumeric) {
                    probes.insert(prefix);
                }
                let (_, initials) = pinyin_of(field);
                if initials.chars().count() >= 2 {
                    probes.insert(initials.chars().take(2).collect::<String>());
                }
            }
        }

        for query in probes {
            let reference = retrieval::reference_search(
                &index.docs,
                &retrieval::query::parse(&query),
                &[],
                MAX_RESULTS,
            );
            let indexed = search_with_index(&index, &query, &[], MAX_RESULTS);
            for hit in reference {
                if matches!(
                    hit.matched_by.as_str(),
                    "nucleo" | "fuzzy" | "keyword-fuzzy"
                ) {
                    continue;
                }
                assert!(
                    indexed.iter().any(|found| found.item.id == hit.item.id),
                    "query={query:?} lost {} ({}) from system entries before verification",
                    hit.item.name,
                    hit.matched_by
                );
            }
        }
    }

    #[test]
    fn single_letter_pinyin_initial_finds_control_panel() {
        // k → 控制面板（kzmb）应可召回；名称前缀（Kite）仍可排更前
        let apps = vec![
            item("Kite"),
            item("kdnet"),
            item("控制面板"),
            item("记事本"),
        ];
        let hits = search(&apps, "k", &[], TOP_N);
        let names: Vec<_> = hits.iter().map(|h| h.item.name.as_str()).collect();
        assert!(
            names.contains(&"控制面板"),
            "单字母 k 应召回控制面板: {names:?}"
        );
        assert!(!names.contains(&"记事本"), "记事本不应被 k 召回: {names:?}");
        // 名称前缀优先于拼音首字母前缀
        assert_eq!(hits[0].item.name, "Kite");
    }

    #[test]
    fn system_tool_control_panel_via_single_letter() {
        let apps = vec![item("Notepad")];
        let entries = crate::app::builtin::materialize_system_entries(None);
        let hits = search_with_system(&apps, &entries, "k", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "控制面板"),
            "系统入口控制面板应被 k 召回: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
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

    // ── 索引多路召回：中段片段 / 跳字 / 混合拼音 / 系统入口同层 ──

    #[test]
    fn mid_string_continuous_fragment() {
        // 名称中间连续片段：ownloa ⊂ Download
        let apps = vec![
            item("Neat Download Manager"),
            item("Downhill Bike"),
            item("Notepad"),
        ];
        let hits = search(&apps, "ownloa", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "Neat Download Manager"),
            "中段片段应召回: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ordered_skip_recalls_omitted_chars() {
        // 有序跳字：googchrome（≥4 字）跳过 google 中的 l/e 与空格
        let apps = vec![item("Google Chrome"), item("Firefox"), item("Steam")];
        let hits = search(&apps, "googchrome", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "Google Chrome"),
            "跳字应召回: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mixed_pinyin_full_and_initial() {
        // wxin：简拼 w + 全拼 xin
        let apps = vec![item("微信"), item("微博"), item("记事本")];
        let hits = search(&apps, "wxin", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "微信"),
            "混合拼音应召回微信: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
        let hits = search(&apps, "weixin", &[], TOP_N);
        assert_eq!(hits[0].item.name, "微信");
    }

    #[test]
    fn non_adjacent_words_in_order() {
        // 不相邻词按序：visual … code
        let apps = vec![
            item("Visual Studio Code"),
            item("Visual Studio"),
            item("Code Blocks"),
        ];
        let hits = search(&apps, "visual code", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "Visual Studio Code"),
            "不相邻词应召回: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn system_entries_share_scoring_with_apps() {
        let apps = vec![item("Notepad")];
        let mut sys = item("显示");
        sys.id = "winsettings:ms-settings:display".into();
        sys.source = "win-settings".into();
        sys.target = "ms-settings:display".into();
        sys.search_keywords = vec!["display".into(), "xianshi".into()];
        sys.attach_search_fields();

        let hits = search_with_system(&apps, &[sys], "显示", &[], TOP_N);
        assert!(
            hits.iter()
                .any(|h| h.item.id == "winsettings:ms-settings:display"),
            "系统入口应与应用统一召回: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
        // 无关 Query 不应把系统入口抬到前面
        let hits = search_with_system(&apps, &[item("显示")], "notepad", &[], TOP_N);
        assert_eq!(hits[0].item.name, "Notepad");
    }

    #[test]
    fn index_matches_reference_on_fixed_queries() {
        // 预过滤允许假阳性，但最终 Top1 应与无剪枝参考一致
        let apps = vec![
            item("Google Chrome"),
            item("Chrome Remote Desktop"),
            item("Visual Studio Code"),
            item("Visual Studio"),
            item("微信"),
            item("Neat Download Manager"),
            item("Microsoft To Do"),
            item("XTerminal"),
        ];
        let queries = [
            "chrome", "vis", "vsc", "微信", "weixin", "ndm", "todo", "ter", "ownloa", "chorme",
        ];
        let index = RetrievalIndex::build(&apps, &[]);
        for q in queries {
            let indexed = search_with_index(&index, q, &[], TOP_N);
            let reference =
                retrieval::reference_search(&index.docs, &retrieval::query::parse(q), &[], TOP_N);
            let indexed_top = indexed.first().map(|h| h.item.name.as_str());
            let ref_top = reference.first().map(|h| h.item.name.as_str());
            assert_eq!(
                indexed_top, ref_top,
                "query={q} indexed vs reference top mismatch"
            );
        }
    }

    #[test]
    fn short_inner_character_and_polyphonic_candidates_match_reference() {
        let apps = vec![
            item("Notepad"),
            item("Google Chrome"),
            item("控制面板"),
            item("重庆"),
        ];
        let index = RetrievalIndex::build(&apps, &[]);
        for (query, expected) in [
            ("e", "Notepad"),
            ("c", "Google Chrome"),
            ("制", "控制面板"),
            ("chong", "重庆"),
            ("ch", "重庆"),
        ] {
            let reference = retrieval::reference_search(
                &index.docs,
                &retrieval::query::parse(query),
                &[],
                TOP_N,
            );
            assert!(reference.iter().any(|hit| hit.item.name == expected));
            let indexed = search_with_index(&index, query, &[], TOP_N);
            assert!(
                indexed.iter().any(|hit| hit.item.name == expected),
                "query={query} lost {expected} before verification"
            );
        }
    }

    #[test]
    fn additional_search_fields_are_tokenized_and_support_inner_fragments() {
        let mut app = item("pwsh");
        app.search_keywords = vec![
            "PowerShell 7".into(),
            "Microsoft Corporation".into(),
            "飞书会议".into(),
        ];
        let index = RetrievalIndex::build(&[app], &[]);

        for query in ["powershell", "corporation", "书会"] {
            let hits = search_with_index(&index, query, &[], TOP_N);
            assert!(
                hits.iter().any(|hit| hit.item.name == "pwsh"),
                "additional field fragment {query:?} must recall its application"
            );
        }
    }

    #[test]
    fn short_query_does_not_flood_fuzzy() {
        let apps: Vec<_> = (0..40)
            .map(|i| item(&format!("Sample Application {i:02}")))
            .collect();
        let hits = search(&apps, "sa", &[], TOP_N);
        // 短 Query 仍可命中前缀，但不应把大量无关项挤满且全部靠 fuzzy
        assert!(hits.len() <= TOP_N);
        assert!(
            hits.iter().all(|h| h.matched_by != "fuzzy"),
            "两字符 Query 不应触发 fuzzy: {:?}",
            hits.iter().map(|h| &h.matched_by).collect::<Vec<_>>()
        );
    }

    #[test]
    fn snapshot_update_reindexes_new_app() {
        let apps = vec![item("Alpha")];
        let hits = search(&apps, "beta", &[], TOP_N);
        assert!(hits.iter().all(|h| h.item.name != "Beta"));

        let apps = vec![item("Alpha"), item("Beta")];
        let hits = search(&apps, "beta", &[], TOP_N);
        assert_eq!(hits[0].item.name, "Beta");
    }

    #[test]
    fn builtin_materialize_has_keywords_and_kite_settings() {
        let entries = crate::app::builtin::materialize_system_entries(None);
        assert!(
            entries.iter().any(|e| e.id == "kite:settings"),
            "应包含 Kite 设置"
        );
        let display = entries
            .iter()
            .find(|e| e.id.starts_with("winsettings:ms-settings:display"));
        assert!(display.is_some(), "应包含显示设置页");
        assert!(
            display
                .unwrap()
                .search_keywords
                .iter()
                .any(|k| k == "display" || k == "xianshi"),
            "关键词应入条目"
        );
    }

    fn named_item(id: &str, name: &str) -> AppItem {
        let mut it = AppItem::scanned(
            id.to_string(),
            name.to_string(),
            format!("C:\\apps\\{id}.exe"),
            None,
            None,
            "start-menu",
        );
        it.attach_search_fields();
        it
    }

    #[test]
    fn personalized_search_keeps_explicit_quality_tier() {
        let apps = vec![named_item("chrome", "Google Chrome")];
        let index = RetrievalIndex::build(&apps, &[]);
        let hits = search_with_personalization(&index, "chrome", &[], None, MAX_RESULTS);
        assert!(!hits.is_empty());
        assert!(
            hits[0].quality_tier > 0,
            "统一入口应写入显式质量层，score={}",
            hits[0].score
        );
    }

    #[test]
    fn preference_lifts_similar_quality_before_truncate() {
        // 两条同质量前缀命中；max_results=1 时旧流程会先截断再加分，
        // 统一入口应在截断前应用 Query 偏好。
        let apps = vec![
            named_item("alpha", "Alpine Tool"),
            named_item("beta", "Albatross Tool"),
        ];
        let index = RetrievalIndex::build(&apps, &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.query_norm = "al".into();
        prefs.pairs.insert(
            "beta".into(),
            crate::storage::QueryPairStats {
                count: 20,
                last_used_at: 1,
            },
        );
        prefs.usage.insert(
            "beta".into(),
            crate::storage::UsageStats {
                launch_count: 50,
                last_used_at: 1_700_000_000,
            },
        );
        prefs.now = 1_700_086_400;

        let hits = search_with_personalization(&index, "al", &[], Some(&prefs), 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].item.id, "beta",
            "同层候选在截断前应被本 Query 偏好抬升"
        );
        assert!(hits[0].matched_by.contains("history"));
    }

    #[test]
    fn unified_entry_protects_name_exact_from_history() {
        let apps = vec![
            named_item("exact", "al"),
            named_item("prefix", "Alpha Tool"),
        ];
        // exact 名称 "al" 精确；prefix 有极强历史也不得抢第一
        let index = RetrievalIndex::build(&apps, &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.query_norm = "al".into();
        prefs.pairs.insert(
            "prefix".into(),
            crate::storage::QueryPairStats {
                count: 100,
                last_used_at: 1,
            },
        );
        prefs.usage.insert(
            "prefix".into(),
            crate::storage::UsageStats {
                launch_count: 10_000,
                last_used_at: 1_700_000_000,
            },
        );
        prefs.now = 1_700_086_400;

        let hits = search_with_personalization(&index, "al", &[], Some(&prefs), MAX_RESULTS);
        assert_eq!(hits[0].item.id, "exact", "Name Exact 不得被历史抬升的弱匹配压过");
        assert!(
            hits[0].quality_tier < hits[1].quality_tier,
            "质量层应显式区分 exact 与 prefix"
        );
    }

    #[test]
    fn same_score_prefers_earlier_match_over_smaller_id() {
        // 两者均为 substring 同分：XcodeX 中 code@1，Abcode 中 code@2。
        // 证据更优的 zzz 应排在 id 更小的 aaa 之前。
        let apps = vec![named_item("zzz", "XcodeX"), named_item("aaa", "Abcode")];
        let index = RetrievalIndex::build(&apps, &[]);
        let hits = search_with_personalization(&index, "code", &[], None, MAX_RESULTS);
        assert!(
            hits.len() >= 2,
            "两条 substring 候选都应召回: {:?}",
            hits.iter().map(|h| &h.item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "zzz",
            "同分应按更早命中起点排序，不得被更小 id 覆盖: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn same_score_prefers_earlier_match_with_empty_personalization() {
        let apps = vec![named_item("zzz", "XcodeX"), named_item("aaa", "Abcode")];
        let index = RetrievalIndex::build(&apps, &[]);
        let prefs = crate::history::Personalization::default();
        let hits = search_with_personalization(&index, "code", &[], Some(&prefs), MAX_RESULTS);
        assert!(
            hits.len() >= 2,
            "两条 substring 候选都应召回: {:?}",
            hits.iter().map(|h| &h.item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "zzz",
            "空个性化路径也必须用同一证据比较键: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pin_does_not_stack_over_exact_in_unified_entry() {
        let apps = vec![
            named_item("exact", "chrome"),
            named_item("pinned", "chromium"),
        ];
        let index = RetrievalIndex::build(&apps, &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.query_norm = "chrome".into();
        prefs.pinned.insert("pinned".into());

        let hits = search_with_personalization(&index, "chrome", &[], Some(&prefs), MAX_RESULTS);
        assert_eq!(hits[0].item.id, "exact");
    }

    #[test]
    fn single_char_hits_inner_name_and_pinyin_initial() {
        let apps = vec![
            named_item("notepad", "Notepad"),
            named_item("control", "控制面板"),
            named_item("other", "Steam"),
        ];
        // 名称内部 n
        let hits = search(&apps, "n", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "notepad"),
            "1 字符 n 应命中 Notepad: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
        // 拼音首字母任意位置 z ← 控制面板 kzmb
        let hits = search(&apps, "z", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "control"),
            "1 字符 z 应命中控制面板: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
        // 1 字符不启用纠错：不应因编辑距离召回无关项
        let hits = search(&apps, "x", &[], TOP_N);
        assert!(
            hits.iter().all(|h| h.matched_by != "fuzzy" && h.matched_by != "keyword-fuzzy"),
            "1 字符不应走 fuzzy"
        );
    }

    #[test]
    fn short_fragment_and_acronym_recall() {
        let apps = vec![
            named_item("notepad", "Notepad"),
            named_item("ndm", "Neat Download Manager"),
            named_item("chrome", "Google Chrome"),
        ];
        let hits = search(&apps, "pad", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "notepad"),
            "pad 应召回 Notepad"
        );
        let hits = search(&apps, "ndm", &[], TOP_N);
        assert_eq!(hits[0].item.id, "ndm", "ndm 缩写应命中");
    }

    #[test]
    fn mixed_hanzi_pinyin_query_recalls() {
        let apps = vec![named_item("wechat", "微信"), named_item("dev", "微信开发者工具")];
        let hits = search(&apps, "微信xin", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "wechat"),
            "微信xin 混输应召回微信: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
        let hits = search(&apps, "w微", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "wechat"),
            "w微 混输应召回微信: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn contiguous_fragment_beats_scattered_same_chars_at_equal_score_band() {
        // googchrome 有序跳字 vs 更晚的同等 substring：证据应偏好更早/更紧
        let apps = vec![
            named_item("gchrome", "gxxchrome"),
            named_item("gchrome2", "xxgchrome"),
        ];
        let hits = search(&apps, "gchrome", &[], TOP_N);
        assert!(
            !hits.is_empty(),
            "gchrome 应至少召回一条有序跳字/片段"
        );
    }

    #[test]
    fn deletion_typo_direct_variant_still_recalls_original() {
        // chrome 的删除变体 crome：查询侧删除深度为 0，不得被纠错通道丢掉。
        let apps = vec![named_item("chrome", "Google Chrome")];
        let hits = search(&apps, "crome", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "chrome"),
            "crome 应召回 Google Chrome: {:?}",
            hits.iter()
                .map(|h| (&h.item.id, &h.matched_by))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn mixed_ordered_alignment_scores_higher_than_loose_fragments() {
        let apps = vec![named_item("wechat", "微信")];
        let ordered = search(&apps, "微信xin", &[], TOP_N);
        let loose = search(&apps, "xin微信", &[], TOP_N);
        assert!(
            ordered.iter().any(|h| h.item.id == "wechat"),
            "微信xin 应召回: {:?}",
            ordered.iter().map(|h| &h.matched_by).collect::<Vec<_>>()
        );
        assert!(
            loose.iter().any(|h| h.item.id == "wechat"),
            "xin微信 应宽召回: {:?}",
            loose.iter().map(|h| &h.matched_by).collect::<Vec<_>>()
        );
        let o = ordered.iter().find(|h| h.item.id == "wechat").unwrap();
        let l = loose.iter().find(|h| h.item.id == "wechat").unwrap();
        assert!(
            o.score > l.score,
            "有序混输分应高于宽召回: ordered={} {} vs loose={} {}",
            o.score,
            o.matched_by,
            l.score,
            l.matched_by
        );
        assert!(
            o.matched_by.contains("ordered"),
            "有序应标记 ordered: {}",
            o.matched_by
        );
        assert!(
            l.matched_by.contains("loose") || l.score < o.score,
            "宽召回不得与有序同权: {}",
            l.matched_by
        );
    }

    #[test]
    fn token_seq_prefers_tighter_name() {
        let apps = vec![
            named_item("tight", "Visual Code"),
            named_item("loose", "Visual Studio Code"),
        ];
        let hits = search(&apps, "visual code", &[], TOP_N);
        assert!(
            hits.len() >= 2,
            "两条 token-seq 都应召回: {:?}",
            hits.iter().map(|h| &h.item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "tight",
            "同分 token-seq 应偏好更紧名称: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }
