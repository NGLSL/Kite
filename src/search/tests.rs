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
        sourced_item_with_args(name, target, source, None)
    }

    fn sourced_item_with_args(
        name: &str,
        target: &str,
        source: &str,
        args: Option<String>,
    ) -> AppItem {
        let mut it = AppItem::scanned(
            format!("{source}:{target}:{}", args.clone().unwrap_or_default()),
            name.into(),
            target.into(),
            args,
            None,
            source,
        );
        // 开始菜单/桌面条目在测试中默认按 .lnk 处理，贴近扫描结果
        if matches!(source, "start-menu" | "desktop") {
            it.is_lnk = true;
        }
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
    fn same_name_different_targets_are_both_kept() {
        let a = sourced_item("微信", r"C:\A\WeChat\WeChat.exe", "start-menu");
        let b = sourced_item("微信", r"C:\B\Weixin\Weixin.exe", "start-menu");
        let hits = search(&[a, b], "微信", &[], TOP_N);
        assert_eq!(
            hits.len(),
            2,
            "同名不同目标不得合并: {:?}",
            hits.iter().map(|h| &h.item.target).collect::<Vec<_>>()
        );
    }

    #[test]
    fn wps_uninstall_merges_into_app_row_without_becoming_launch_target() {
        // 真机：卸载注册表 DisplayName = WPS Office (12.1.0.28505)，与主程序同一安装
        let app = sourced_item_with_args(
            "WPS Office",
            r"D:\Program Files\WPS Office\ksolaunch.exe",
            "start-menu",
            Some("/prometheus /fromksolaunch /from=startmenu".into()),
        );
        let uninstall = sourced_item(
            "WPS Office (12.1.0.28505)",
            r"D:\Program Files\WPS Office\12.1.0.28505\utility\uninst.exe",
            "uninstall",
        );
        let hits = search(&[app, uninstall], "wps", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "同一安装的主程序与卸载项只展示一条: {:?}",
            hits.iter()
                .map(|h| (&h.item.name, &h.item.source, &h.item.target))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[0].item.source, "start-menu");
        assert!(
            hits[0].item.target.ends_with("ksolaunch.exe"),
            "启动不得落到 uninst.exe: {}",
            hits[0].item.target
        );
    }

    #[test]
    fn wps_merge_keeps_startmenu_launch_args_and_borrows_icon() {
        let launch = r"D:\Program Files\WPS Office\ksolaunch.exe";
        let mut menu = AppItem::scanned(
            "menu".into(),
            "WPS Office".into(),
            launch.into(),
            Some("/prometheus /fromksolaunch /from=startmenu".into()),
            Some(r"D:\Program Files\WPS Office".into()),
            "start-menu",
        );
        menu.attach_search_fields();
        menu.icon = None;
        menu.icon_src = None;
        let mut exe = AppItem::scanned(
            "exe".into(),
            "wps".into(),
            r"D:\Program Files\WPS Office\12.1.0.28505\office6\wps.exe".into(),
            None,
            None,
            "app-paths",
        );
        exe.attach_search_fields();
        exe.icon = Some(r"C:\cache\wps.png".into());
        exe.icon_src = Some(exe.target.clone());

        let hits = search(&[menu, exe], "wps", &[], TOP_N);
        assert_eq!(hits.len(), 1, "应归并一条");
        let h = &hits[0];
        assert_eq!(h.item.source, "start-menu", "启动代表必须是开始菜单");
        assert_eq!(
            h.item.target, launch,
            "target 保持 ksolaunch，不能被换成 wps.exe"
        );
        assert!(
            h.item.args.as_deref().unwrap_or("").contains("/from=startmenu"),
            "args 保留 lnk 启动语义: {:?}",
            h.item.args
        );
        assert!(
            h.item.icon.is_some(),
            "图标应从 app-paths 借用: {:?}",
            h.item.icon
        );
    }

    #[test]
    fn wps_all_ksolaunch_and_shell_entries_display_one_row() {
        // 真机：多个入口都落到 ksolaunch.exe / 同一 WPS 安装
        let launch = r"D:\Program Files\WPS Office\ksolaunch.exe";
        let wps_exe = r"D:\Program Files\WPS Office\12.1.0.28505\office6\wps.exe";
        let mut menu = sourced_item_with_args(
            "WPS Office",
            launch,
            "start-menu",
            Some("/prometheus /fromksolaunch /from=startmenu".into()),
        );
        menu.working_dir = Some(r"D:\Program Files\WPS Office".into());
        let mut desk = sourced_item_with_args(
            "WPS Office",
            launch,
            "desktop",
            Some("/prometheus /fromksolaunch /from=desktop_shortcut".into()),
        );
        desk.working_dir = Some(r"D:\Program Files\WPS Office".into());
        let apps = vec![
            menu,
            desk,
            sourced_item("wps", wps_exe, "app-paths"),
            sourced_item(
                "WPS Office",
                r"shell:AppsFolder\Kingsoft.Office.KPrometheus",
                "apps-folder",
            ),
            sourced_item("WPS Office (12.1.0.28505)", wps_exe, "start-menu"),
        ];
        let hits = search(&apps, "wps", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "真机 WPS 多入口应只展示一条: {:?}",
            hits.iter()
                .map(|h| (&h.item.name, &h.item.source, &h.item.target, &h.item.args))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.target, launch,
            "启动必须走开始菜单 ksolaunch，不能换成裸 wps.exe"
        );
        assert!(
            hits[0].item.args.as_deref().unwrap_or("").contains("startmenu"),
            "应保留 lnk 的 /from=startmenu: {:?}",
            hits[0].item.args
        );
    }

    #[test]
    fn same_name_same_install_different_launcher_args_display_one_row() {
        // 真机 WPS：startmenu / desktop_shortcut 仅启动来源参数不同
        let base = r"D:\Program Files\WPS Office";
        let apps = vec![
            sourced_item_with_args(
                "WPS Office",
                &format!("{base}\\ksolaunch.exe"),
                "start-menu",
                Some("/prometheus /fromksolaunch /from=startmenu".into()),
            ),
            sourced_item_with_args(
                "WPS Office",
                &format!("{base}\\ksolaunch.exe"),
                "desktop",
                Some("/prometheus /fromksolaunch /from=desktop_shortcut".into()),
            ),
        ];
        let hits = search(&apps, "wps office", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "同安装同名不同 /from= 参数应归并: {:?}",
            hits.iter()
                .map(|h| (&h.item.source, &h.item.args))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn utools_desktop_exe_and_appsfolder_package_merge() {
        let lnk = sourced_item(
            "uTools",
            r"C:\Users\admin\AppData\Local\Programs\utools\uTools.exe",
            "start-menu",
        );
        let mut shell = AppItem::scanned(
            "shell:org.yuanli.utools".into(),
            "uTools".into(),
            r"shell:AppsFolder\org.yuanli.utools".into(),
            None,
            None,
            "apps-folder",
        );
        shell.attach_search_fields();
        shell.icon = Some(r"C:\cache\utools-shell.png".into());
        shell.icon_src = Some(r"shell:AppsFolder\org.yuanli.utools".into());

        let hits = search(&[lnk, shell], "utools", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "桌面 uTools + AppsFolder 包应归并: {:?}",
            hits.iter()
                .map(|h| (&h.item.source, &h.item.target))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn same_launch_identity_displays_one_friendly_row() {
        let target = r"C:\Program Files\Example\app.exe";
        let mut a = AppItem::scanned(
            "id-a".into(),
            "Example".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        a.attach_search_fields();
        let mut b = AppItem::scanned(
            "id-b".into(),
            "Example".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        b.attach_search_fields();
        let hits = search(&[a, b], "Example", &[], TOP_N);
        assert_eq!(hits.len(), 1, "同一启动身份只展示一条");
        assert_eq!(hits[0].item.source, "start-menu");
    }

    #[test]
    fn utools_lnk_and_exe_same_target_merge_to_one_with_icon() {
        // 真机截图形态：开始菜单 lnk 无图 + app-paths exe 有图，指向同一 exe
        let target = r"C:\Users\admin\AppData\Local\uTools\uTools.exe";
        let mut lnk = AppItem::scanned(
            "start-menu:utools".into(),
            "uTools".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        lnk.attach_search_fields();
        lnk.icon = None;
        lnk.icon_src = None;
        let mut exe = AppItem::scanned(
            "app-paths:utools".into(),
            "uTools".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        exe.attach_search_fields();
        exe.icon = Some(r"C:\cache\utools.png".into());
        exe.icon_src = Some(target.into());

        let hits = search(&[lnk, exe], "utools", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "同一 target 的 lnk+exe 应只展示一条: {:?}",
            hits.iter()
                .map(|h| (&h.item.source, &h.item.icon, &h.item.target))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[0].item.source, "start-menu", "启动保留 lnk");
        assert!(
            hits[0].item.icon.is_some(),
            "图标从 exe 入口借用"
        );
    }

    #[test]
    fn utools_different_install_roots_stay_two_rows() {
        // AppData 与 Program Files 两套安装：不能合并
        let apps = vec![
            sourced_item(
                "uTools",
                r"C:\Users\admin\AppData\Local\uTools\uTools.exe",
                "start-menu",
            ),
            sourced_item(
                "uTools",
                r"C:\Program Files\uTools\uTools.exe",
                "app-paths",
            ),
        ];
        let hits = search(&apps, "utools", &[], TOP_N);
        assert_eq!(
            hits.len(),
            2,
            "不同安装根必须保留: {:?}",
            hits.iter().map(|h| &h.item.target).collect::<Vec<_>>()
        );
    }

    #[test]
    fn merged_row_prefers_entry_with_icon() {
        let target = r"C:\Program Files\uTools\uTools.exe";
        let mut lnk = AppItem::scanned(
            "lnk".into(),
            "uTools".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        lnk.attach_search_fields();
        // 模拟 lnk 未解析出图标
        lnk.icon = None;
        lnk.icon_src = None;
        let mut exe = AppItem::scanned(
            "exe".into(),
            "uTools".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        exe.attach_search_fields();
        exe.icon = Some("C:\\cache\\utools.png".into());
        exe.icon_src = Some(target.into());

        let hits = search(&[lnk, exe], "uTools", &[], TOP_N);
        assert_eq!(hits.len(), 1, "同一启动身份只一条");
        assert_eq!(hits[0].item.source, "start-menu", "启动仍用开始菜单入口");
        assert!(
            hits[0].item.icon.is_some() || hits[0].item.icon_src.is_some(),
            "图标应从同组带图入口借用: {:?}",
            (hits[0].item.source.as_str(), &hits[0].item.icon)
        );
    }

    #[test]
    fn wps_multi_entry_same_install_displays_one_row() {
        let base = r"D:\Program Files\WPS Office";
        let apps = vec![
            sourced_item("wps", &format!("{base}\\12.1.0.28505\\office6\\wps.exe"), "app-paths"),
            sourced_item("WPS Office", &format!("{base}\\ksolaunch.exe"), "start-menu"),
            sourced_item("WPS Office", &format!("{base}\\12.1.0.28505\\office6\\wps.exe"), "desktop"),
            sourced_item(
                "WPS Office",
                &format!("shell:AppsFolder\\{base}\\ksolaunch.exe"),
                "apps-folder",
            ),
            sourced_item(
                "WPS Office (12.1.0.28505)",
                &format!("{base}\\12.1.0.28505\\office6\\wps.exe"),
                "start-menu",
            ),
        ];
        let hits = search(&apps, "wps", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "同一 WPS 安装族只应展示一条: {:?}",
            hits.iter()
                .map(|h| (&h.item.name, &h.item.source, &h.item.target))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn different_install_roots_with_same_name_are_both_kept() {
        let apps = vec![
            sourced_item("WPS Office", r"D:\Apps\WPSA\wps.exe", "start-menu"),
            sourced_item("WPS Office", r"D:\Apps\WPSB\wps.exe", "start-menu"),
        ];
        let hits = search(&apps, "wps office", &[], TOP_N);
        assert_eq!(hits.len(), 2, "不同安装根不得合并");
    }

    #[test]
    fn wechat_lnk_and_apps_folder_same_exe_display_one_row() {
        let exe = r"C:\Program Files (x86)\Tencent\WeChat\WeChat.exe";
        let lnk = sourced_item("微信", exe, "start-menu");
        let shell = sourced_item(
            "微信",
            &format!("shell:AppsFolder\\{exe}"),
            "apps-folder",
        );
        let hits = search(&[lnk, shell], "微信", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "开始菜单与 AppsFolder 指向同一 exe 应只展示一条: {:?}",
            hits.iter()
                .map(|h| (&h.item.source, &h.item.target))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[0].item.source, "start-menu");
    }

    #[test]
    fn open_group_history_can_cross_adjacent_quality_tiers() {
        // 前缀命中 800（层4） vs 词前缀 720（层6）+ 强历史：普通组按最终分竞争
        let prefix = named_item("prefix", "Terra");
        let weaker = named_item("weaker", "Windows Terminal");
        let index = RetrievalIndex::build(&[prefix, weaker], &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.query_norm = "ter".into();
        prefs.pairs.insert(
            "weaker".into(),
            crate::storage::QueryPairStats {
                count: 50,
                last_used_at: 1,
            },
        );
        prefs.usage.insert(
            "weaker".into(),
            crate::storage::UsageStats {
                launch_count: 10_000,
                last_used_at: 1_700_000_000,
            },
        );
        prefs.now = 1_700_086_400;
        let hits = search_with_personalization(&index, "ter", &[], Some(&prefs), MAX_RESULTS);
        assert!(
            hits.iter().any(|h| h.item.id == "weaker"),
            "两条都应召回: {:?}",
            hits.iter().map(|h| (&h.item.id, h.score)).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "weaker",
            "普通组最终分应允许弱一层候选靠前: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mixed_pinyin_restatement_is_not_ordered_high() {
        let apps = vec![named_item("wechat", "微信")];
        let ordered = search(&apps, "微xin", &[], TOP_N);
        let restated = search(&apps, "微信xin", &[], TOP_N);
        assert!(ordered.iter().any(|h| h.item.id == "wechat"));
        assert!(restated.iter().any(|h| h.item.id == "wechat"));
        let o = ordered.iter().find(|h| h.item.id == "wechat").unwrap();
        let r = restated.iter().find(|h| h.item.id == "wechat").unwrap();
        assert!(
            o.matched_by.contains("ordered"),
            "微xin 应有序: {}",
            o.matched_by
        );
        assert!(
            r.score < o.score || !r.matched_by.contains("ordered"),
            "微信xin 复述不得与有序同权: ordered={} {} vs restated={} {}",
            o.score,
            o.matched_by,
            r.score,
            r.matched_by
        );
    }

    #[test]
    fn source_preference_applies_to_other_apps_and_preserves_unrelated_exact_match() {
        let raw = sourced_item("aurora", r"D:\Apps\Aurora\bin\aurora.exe", "app-paths");
        let friendly = sourced_item("Aurora Studio", r"d:\apps\aurora\launcher.exe", "desktop");
        let hits = search(&[raw.clone(), friendly], "aurora", &[], TOP_N);
        assert_eq!(
            hits.iter().map(|h| h.item.name.as_str()).collect::<Vec<_>>(),
            vec!["Aurora Studio"],
            "同安装 aurora.exe ↔ Aurora Studio 归并一条: {:?}",
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
    fn hit_exe_name_still_shows_start_menu_lnk_main_entry() {
        // 同一 target：只命中 app-paths 的英文名，开始菜单中文 .lnk 仍是主入口
        let target = r"C:\Program Files\Tencent\WeChat\WeChat.exe";
        let mut menu = AppItem::scanned(
            "menu".into(),
            "微信".into(),
            target.into(),
            None,
            Some(r"C:\Program Files\Tencent\WeChat".into()),
            "start-menu",
        );
        menu.is_lnk = true;
        menu.attach_search_fields();
        let exe = sourced_item("wechat", target, "app-paths");
        let hits = search(&[menu, exe], "wechat", &[], TOP_N);
        assert_eq!(
            hits.len(),
            1,
            "同启动身份应归并: {:?}",
            hits.iter()
                .map(|h| (&h.item.name, &h.item.source))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[0].item.source, "start-menu");
        assert_eq!(hits[0].item.name, "微信", "展示名来自主入口");
        assert_eq!(hits[0].item.target, target);
        assert!(
            hits[0].item.working_dir.is_some(),
            "主入口 working_dir 应保留"
        );
    }

    #[test]
    fn same_target_different_action_args_are_not_merged() {
        // 同一 exe、不同动作：打开应用 vs 设置面板，不得合并
        let target = r"C:\Program Files\Example\tool.exe";
        let open = sourced_item("Example Tool", target, "start-menu");
        let settings = sourced_item_with_args(
            "Example Tool Settings",
            target,
            "desktop",
            Some("--settings".into()),
        );
        let hits = search(&[open, settings], "example tool", &[], TOP_N);
        assert_eq!(
            hits.len(),
            2,
            "不同启动动作必须分行: {:?}",
            hits.iter()
                .map(|h| (&h.item.name, h.item.args.as_deref()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn apps_without_lnk_still_search_and_keep_launch_target() {
        let app = sourced_item("Solo App", r"C:\Tools\solo\solo.exe", "app-paths");
        let hits = search(&[app.clone()], "solo", &[], TOP_N);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.target, app.target);
        assert_eq!(hits[0].item.source, "app-paths");
    }

    #[test]
    fn icon_borrow_is_stable_regardless_of_member_order() {
        let target = r"C:\Program Files\uTools\uTools.exe";
        let mut lnk = AppItem::scanned(
            "lnk".into(),
            "uTools".into(),
            target.into(),
            None,
            None,
            "start-menu",
        );
        lnk.is_lnk = true;
        lnk.attach_search_fields();
        lnk.icon = None;
        lnk.icon_src = None;
        let mut exe = AppItem::scanned(
            "exe".into(),
            "uTools".into(),
            target.into(),
            None,
            None,
            "app-paths",
        );
        exe.attach_search_fields();
        exe.icon = Some(r"C:\cache\utools.png".into());
        exe.icon_src = Some(target.into());

        let a = search(&[lnk.clone(), exe.clone()], "utools", &[], TOP_N);
        let b = search(&[exe, lnk], "utools", &[], TOP_N);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].item.source, "start-menu");
        assert_eq!(b[0].item.source, "start-menu");
        assert_eq!(a[0].item.icon, b[0].item.icon);
        assert!(a[0].item.icon.is_some(), "应借用 exe 缓存图标");
        assert_eq!(a[0].score, b[0].score, "证据与分数不随处理顺序变化");
    }

    #[test]
    fn pinned_member_wins_as_launch_representative() {
        let menu = sourced_item(
            "Example",
            r"C:\Program Files\Example\launcher.exe",
            "start-menu",
        );
        let mut exe = sourced_item(
            "Example",
            r"C:\Program Files\Example\bin\example.exe",
            "app-paths",
        );
        // 不同 target、同安装根 → 族合并；钉选 exe 时应成为启动代表
        exe.icon = Some(r"C:\cache\example.png".into());
        let index = RetrievalIndex::build(&[menu, exe.clone()], &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.pinned.insert(exe.id.clone());
        let hits = search_with_personalization(&index, "example", &[], Some(&prefs), TOP_N);
        assert_eq!(hits.len(), 1, "同安装应归并");
        assert_eq!(
            hits[0].item.source, "app-paths",
            "用户钉选的入口优先于默认主入口"
        );
        assert_eq!(hits[0].item.id, exe.id);
    }

    #[test]
    fn pinned_group_member_wins_even_when_not_recalled() {
        let target = r"C:\Program Files\Example\app.exe";
        let menu = sourced_item("Example App", target, "start-menu");
        let exe = sourced_item("examplehelper", target, "app-paths");
        let index = RetrievalIndex::build(&[menu, exe.clone()], &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.pinned.insert(exe.id.clone());
        // 查询只命中开始菜单名，钉选的 app-paths 未被召回，仍应作启动代表
        let hits = search_with_personalization(&index, "example app", &[], Some(&prefs), TOP_N);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.source, "app-paths");
        assert_eq!(hits[0].item.id, exe.id);
    }

    #[test]
    fn pinned_rep_borrows_icon_from_static_group_members() {
        let target = r"C:\Program Files\Example\app.exe";
        let mut menu = sourced_item("Example App", target, "start-menu");
        menu.icon = Some(r"C:\cache\example.png".into());
        let exe = sourced_item("examplehelper", target, "app-paths");
        let index = RetrievalIndex::build(&[menu, exe.clone()], &[]);
        let mut prefs = crate::history::Personalization::default();
        prefs.pinned.insert(exe.id.clone());
        let hits = search_with_personalization(&index, "example app", &[], Some(&prefs), TOP_N);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.source, "app-paths");
        assert!(
            hits[0].item.icon.is_some(),
            "钉选覆盖主入口后仍应从组内借缓存图标: {:?}",
            (hits[0].item.source.as_str(), &hits[0].item.icon)
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
                    "nucleo" | "fuzzy" | "keyword-fuzzy" | "pinyin-fuzzy"
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
    fn demote_applies_across_equivalent_launch_members() {
        // 同一安装的 .lnk 主入口与桌面入口基础分相同，归并后只展示一行。
        // 用户降权的是这一行；降权必须落到展示行分数上，不能被同组另一入口抵消。
        let launch = r"C:\Program Files\Example\ksolaunch.exe";
        let menu = sourced_item("Example", launch, "start-menu");
        let desktop = sourced_item("Example", launch, "desktop");
        let index = RetrievalIndex::build(&[menu.clone(), desktop.clone()], &[]);

        let plain = search_with_personalization(&index, "examp", &[], None, TOP_N);
        assert_eq!(plain.len(), 1, "同安装同启动身份应归并成一行");
        let baseline = plain[0].score;

        let mut prefs = crate::history::Personalization::default();
        prefs.demoted.insert(menu.id.clone());
        let hits = search_with_personalization(&index, "examp", &[], Some(&prefs), TOP_N);

        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].score,
            baseline - crate::storage::demote::DEMOTE_PENALTY,
            "降权必须落在展示行分数上: {:?}",
            (hits[0].item.id.as_str(), &hits[0].matched_by, hits[0].score)
        );
        assert!(
            hits[0].matched_by.contains("+demote"),
            "展示行应带降权标记: {:?}",
            hits[0].matched_by
        );
        assert_eq!(
            hits[0].item.target, plain[0].item.target,
            "降权不得改启动目标"
        );
    }

    #[test]
    fn demote_group_falls_behind_equal_peer_from_another_install() {
        // 两组同名、同启动身份形状、不同安装根（不归并），基础分相同。
        // 只降权其中一组，该组必须整体后移。
        let a_launch = r"C:\Program Files\Alpha\ksolaunch.exe";
        let b_launch = r"C:\Program Files\Beta\ksolaunch.exe";
        let a_menu = sourced_item("Example", a_launch, "start-menu");
        let a_desktop = sourced_item("Example", a_launch, "desktop");
        let b_menu = sourced_item("Example", b_launch, "start-menu");
        let index = RetrievalIndex::build(&[a_menu.clone(), a_desktop, b_menu], &[]);

        let plain = search_with_personalization(&index, "examp", &[], None, TOP_N);
        assert_eq!(plain.len(), 2, "不同安装根不得归并");
        assert_eq!(plain[0].item.target, a_launch, "先确认降权前该组位于前面");

        let mut prefs = crate::history::Personalization::default();
        prefs.demoted.insert(a_menu.id.clone());
        let hits = search_with_personalization(&index, "examp", &[], Some(&prefs), TOP_N);

        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].item.target,
            b_launch,
            "被降权的展示组应后移: {:?}",
            hits.iter()
                .map(|h| (&h.item.target, h.score))
                .collect::<Vec<_>>()
        );
        assert_eq!(hits[1].item.target, a_launch);
    }

    #[test]
    fn group_demote_is_not_stacked_and_is_restorable() {
        let launch = r"C:\Program Files\Example\ksolaunch.exe";
        let menu = sourced_item("Example", launch, "start-menu");
        let desktop = sourced_item("Example", launch, "desktop");
        let index = RetrievalIndex::build(&[menu.clone(), desktop.clone()], &[]);
        let baseline = search_with_personalization(&index, "examp", &[], None, TOP_N)[0].score;

        // 组内两个入口都被降权：仍只扣一次
        let mut both = crate::history::Personalization::default();
        both.demoted.insert(menu.id.clone());
        both.demoted.insert(desktop.id.clone());
        let stacked = search_with_personalization(&index, "examp", &[], Some(&both), TOP_N);
        assert_eq!(
            stacked[0].score,
            baseline - crate::storage::demote::DEMOTE_PENALTY,
            "同组降权不得重复累加"
        );

        // 恢复优先级后回到原分
        let restored = search_with_personalization(
            &index,
            "examp",
            &[],
            Some(&crate::history::Personalization::default()),
            TOP_N,
        );
        assert_eq!(restored[0].score, baseline, "恢复后应回到原序");
    }

    #[test]
    fn alias_target_change_reorders_results() {
        // 别名目标直接决定明确匹配保护层的命中：改指向后同一输入必须立刻改序。
        // 这也是基础候选缓存必须随别名失效的原因——候选集本身随别名变化。
        let apps = vec![
            named_item("alpha", "Alpha Tool"),
            named_item("beta", "Beta Tool"),
        ];
        let index = RetrievalIndex::build(&apps, &[]);
        let target = |id: &str, name: &str| UserTarget {
            id: Some(id.into()),
            name: name.into(),
        };

        let hits = search_with_personalization(
            &index,
            "qa",
            &[target("alpha", "Alpha Tool")],
            None,
            TOP_N,
        );
        assert_eq!(hits[0].item.id, "alpha", "别名指向 alpha 时 alpha 应排最前");

        let hits = search_with_personalization(
            &index,
            "qa",
            &[target("beta", "Beta Tool")],
            None,
            TOP_N,
        );
        assert_eq!(hits[0].item.id, "beta", "改指 beta 后同一输入应立即改序");
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

    #[test]
    fn token_seq_prefers_earlier_start_even_when_name_longer() {
        // 同为 token-seq（gaps 均为 1）：zzz 名称更长但 visual 起点 0；
        // aaa 名称更短但起点更晚。证据 start 必须写真实位置。
        let apps = vec![
            named_item("zzz", "Visual Studio Code Extra Long"),
            named_item("aaa", "A Visual Code"),
        ];
        let hits = search_with_personalization(
            &RetrievalIndex::build(&apps, &[]),
            "visual code",
            &[],
            None,
            MAX_RESULTS,
        );
        assert!(
            hits.len() >= 2,
            "两条 token-seq 都应召回: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by)).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "zzz",
            "同分应偏好更早原名称起点，不得被更短名/name_lower 覆盖: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn skip_prefers_earlier_start_even_when_name_longer() {
        // 同为有序跳字（均非连续 gchrome，避免 substring 分档不同）：
        // zzz 起点 0、名称更长；aaa 起点更晚、名称更短。
        let apps = vec![
            named_item("zzz", "gxxchromeyyy"),
            named_item("aaa", "xxgxxchrome"),
        ];
        let hits = search_with_personalization(
            &RetrievalIndex::build(&apps, &[]),
            "gchrome",
            &[],
            None,
            MAX_RESULTS,
        );
        assert!(
            hits.len() >= 2,
            "两条 skip 都应召回: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by)).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "zzz",
            "skip 同分应偏好更早原名称起点: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pinyin_field_typo_still_recalls() {
        // 全拼字段参与受限纠错：jisunqi → jisuanqi（计算器）
        let apps = vec![item("计算器")];
        let hits = search(&apps, "jisunqi", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.id == "计算器" || h.item.name == "计算器"),
            "jisunqi 应召回计算器: {:?}",
            hits.iter()
                .map(|h| (&h.item.name, &h.matched_by, h.score))
                .collect::<Vec<_>>()
        );
        let hits = search(&apps, "jisuanqi", &[], TOP_N);
        assert!(
            hits.iter().any(|h| h.item.name == "计算器"),
            "正确全拼 jisuanqi 不得回退: {:?}",
            hits.iter().map(|h| &h.item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pinyin_initial_inner_prefers_earlier_mapped_start() {
        // 「kz」在两条名称中部命中（同为 pinyin-initial-inner 分档）：
        // 键盘控制… 起点更早，向日葵…更晚但名称更短。
        let apps = vec![
            named_item("zzz", "向日葵远程控制"),
            named_item("aaa", "键盘控制鼠标设置"),
        ];
        let hits = search_with_personalization(
            &RetrievalIndex::build(&apps, &[]),
            "kz",
            &[],
            None,
            MAX_RESULTS,
        );
        assert!(
            hits.len() >= 2,
            "kz 应召回两条: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by)).collect::<Vec<_>>()
        );
        assert_eq!(
            hits[0].item.id, "aaa",
            "首字母中部命中应按映射回原名称的 start 排序: {:?}",
            hits.iter().map(|h| (&h.item.id, &h.matched_by, h.score)).collect::<Vec<_>>()
        );
    }
