//! 结果组装 / 文件模式 / 直接路径 / 查询日志闸门回归。

use super::super::test_support::{settings_result, test_state};
use super::{
    build_direct_path_results, build_empty_query_lists, build_file_results,
    everything_status_result, is_current_file_response, prepend_dependency_status,
    prepend_direct_path_results,
};
use crate::model::ResultAction;
use crate::system::everything::{self, Availability};
use crate::ui::actions::menu_action;
use crate::ui::MenuAction;

#[test]
fn bootstrap_ready_bumps_index_generation_and_clears_base_cache() {
    use super::super::interaction::update;
    use super::super::Message;

    let mut state = test_state("chrome");
    {
        let mut item = crate::model::AppItem::scanned(
            "chrome".into(),
            "Google Chrome".into(),
            r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
            None,
            None,
            "start-menu",
        );
        item.attach_search_fields();
        let mut index = state.index.lock().unwrap_or_else(|e| e.into_inner());
        index.apps.push(item);
        index.rebuild_retrieval();
    }
    let before_gen = state.index_generation;
    let epoch = state.base_hit_cache.epoch();
    assert!(state
        .base_hit_cache
        .insert_if_epoch(epoch, before_gen, "chrome", vec![]));
    assert!(!state.base_hit_cache.is_empty());
    state.index_ready = false;

    let _ = update(&mut state, Message::BootstrapReady(1));

    assert!(
        state.index_generation == before_gen.wrapping_add(1),
        "Bootstrap 必须提升索引代际"
    );
    assert!(
        state.base_hit_cache.is_empty(),
        "Bootstrap 后不得继续服务旧代际候选缓存"
    );
    assert!(state.index_ready);
}

#[test]
fn non_empty_query_refresh_delegates_to_worker() {
    let mut state = test_state("k");
    let before = state.app_search_worker.latest_seq();

    state.refresh_results();

    assert!(
        state.app_search_worker.latest_seq() > before,
        "非空 Query 必须交给常驻 worker 组装，而不是同步内联一份"
    );
    assert_eq!(state.app_query_generation, 1);
    // 同步调用不再改写列表：结果只由 AppSearchReady 落地。
    assert_eq!(state.results.len(), 1);
    assert_eq!(state.results[0].item.id, "kite:settings");
}

#[test]
fn empty_query_refresh_stays_synchronous() {
    let mut state = test_state("");

    state.refresh_results();

    assert!(
        state.results.is_empty(),
        "空 Query 同步重建列表（空索引 + 无历史）"
    );
    assert!(!state.results_stale, "空 Query 的同步列表就是当前状态");
    assert_eq!(
        state.app_search_worker.latest_seq(),
        1,
        "空 Query 不提交任务，只推进一次失效代际让在途搜索停下"
    );
}

#[test]
fn clearing_query_cancels_in_flight_app_search() {
    let mut state = test_state("k");
    state.refresh_results();
    let submitted = state.app_search_worker.latest_seq();
    let stale_generation = state.app_query_generation;
    let index_generation = state.index_generation;

    state.query.clear();
    state.refresh_results();

    assert!(
        state.app_search_worker.latest_seq() > submitted,
        "清空输入必须通知 worker 停止当前任务"
    );
    assert!(!state.results_stale, "清空后同步列表即当前状态");

    // 清空后即便在途任务仍把旧结果送回来，也不得落地。
    state.apply_app_search_ready(
        stale_generation,
        "k".into(),
        vec![settings_result()],
        0,
        index_generation,
    );
    assert!(
        state.results.is_empty(),
        "被取消的旧任务不得改变清空后的列表"
    );
}

#[test]
fn hiding_window_cancels_in_flight_app_search() {
    use crate::ui::actions::hide;

    let mut state = test_state("k");
    state.refresh_results();
    let submitted = state.app_search_worker.latest_seq();

    hide(&mut state);

    assert!(state.hidden);
    assert!(
        state.app_search_worker.latest_seq() > submitted,
        "隐藏窗口必须通知 worker 停止当前任务"
    );
}

#[test]
fn pin_toggle_on_non_empty_query_routes_through_worker() {
    let mut state = test_state("k");
    let item = state.results[0].item.clone();
    let before = state.app_search_worker.latest_seq();

    let _ = menu_action(&mut state, item, MenuAction::TogglePin);

    assert!(
        state.app_search_worker.latest_seq() > before,
        "Pin 与 Demote 必须共用同一条刷新路径（worker）"
    );
}

#[test]
fn stale_or_disabled_file_results_are_rejected() {
    assert!(is_current_file_response(true, 4, "report", 4, "report"));
    assert!(!is_current_file_response(true, 5, "report", 4, "report"));
    assert!(!is_current_file_response(true, 4, "reports", 4, "report"));
    assert!(!is_current_file_response(false, 4, "report", 4, "report"));
}

#[test]
fn files_mode_ranks_everything_hits_before_apps() {
    let mut state = test_state("test");
    state.files_mode = true;
    state.query = "test".into();

    let mut file_item = crate::model::AppItem::scanned(
        "file:c:\\tools\\test.png".into(),
        "test.png".into(),
        r"C:\Tools\test.png".into(),
        None,
        None,
        "everything",
    );
    file_item.attach_search_fields();
    let file_hit = crate::model::SearchResult::scored(file_item, 400, "file");
    state.file_results = vec![file_hit];

    let mut app_item = crate::model::AppItem::scanned(
        "app:test".into(),
        "test runner".into(),
        r"C:\Apps\test.exe".into(),
        None,
        None,
        "start-menu",
    );
    app_item.attach_search_fields();
    let app_hit = crate::model::SearchResult::scored(app_item, 900, "exact");

    let generation = state.app_query_generation;
    let index_generation = state.index_generation;
    state.apply_app_search_ready(generation, "test".into(), vec![app_hit], 0, index_generation);

    assert!(!state.results.is_empty(), "文件模式下合并结果不得为空");
    assert_eq!(
        state.results[0].item.source, "everything",
        "文件模式下 Everything 命中必须排在应用之前，实际 top={:?}",
        state.results.first().map(|r| &r.item.display_name)
    );
    assert!(
        state.results.iter().any(|r| r.item.source == "start-menu"),
        "文件优先不移除应用命中"
    );
    assert!(
        state.results
            .iter()
            .all(|r| r.item.source != "websearch" && r.item.source != "browser"),
        "文件模式已有文件结果时不得再插入网页搜索"
    );
}

#[test]
fn non_files_mode_still_appends_file_hits_after_apps() {
    let mut state = test_state("test");
    state.files_mode = false;
    state.query = "test".into();

    let mut file_item = crate::model::AppItem::scanned(
        "file:c:\\tools\\test.png".into(),
        "test.png".into(),
        r"C:\Tools\test.png".into(),
        None,
        None,
        "everything",
    );
    file_item.attach_search_fields();
    state.file_results = vec![crate::model::SearchResult::scored(file_item, 400, "file")];

    let mut app_item = crate::model::AppItem::scanned(
        "app:test".into(),
        "test runner".into(),
        r"C:\Apps\test.exe".into(),
        None,
        None,
        "start-menu",
    );
    app_item.attach_search_fields();
    let app_hit = crate::model::SearchResult::scored(app_item, 900, "exact");

    let generation = state.app_query_generation;
    let index_generation = state.index_generation;
    state.apply_app_search_ready(generation, "test".into(), vec![app_hit], 0, index_generation);

    assert_eq!(
        state.results.first().map(|r| r.item.source.as_str()),
        Some("start-menu"),
        "非文件模式不应把未请求的文件结果抬到应用前"
    );
    assert!(
        state.results.iter().all(|r| r.item.source != "everything"),
        "非文件模式不得合并 file_results"
    );
}

#[test]
fn changing_file_filter_invalidates_previous_query() {
    use super::super::interaction::update;
    use super::super::Message;

    let mut state = test_state("Kite");
    state.files_mode = true;
    state.file_query_generation = 4;
    let _ = update(
        &mut state,
        Message::FileFilterChanged(everything::FileFilter::Images),
    );

    assert_eq!(state.query, "Kite");
    assert_eq!(state.file_filter, everything::FileFilter::Images);
    assert_eq!(state.file_query_generation, 5);
    assert!(!is_current_file_response(
        true,
        state.file_query_generation,
        &state.query,
        4,
        "Kite"
    ));
}

#[test]
fn missing_everything_is_visible_as_a_search_result() {
    if everything::availability() != Availability::NotInstalled {
        return;
    }

    let results = build_file_results(
        "logo",
        &std::env::temp_dir(),
        everything::FileFilter::Images,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].item.id, "kite:everything-download");
    assert!(results[0].item.display_name.contains("未安装 Everything"));
}

#[test]
fn dependency_status_distinguishes_missing_and_not_running() {
    let missing = everything_status_result(Availability::NotInstalled).expect("missing status");
    assert_eq!(missing.item.id, everything::DOWNLOAD_RESULT_ID);
    assert_eq!(missing.item.target, everything::DOWNLOAD_URL);
    assert!(missing.item.display_name.contains("官方下载"));

    let stopped =
        everything_status_result(Availability::InstalledButNotRunning).expect("not-running status");
    assert_eq!(stopped.item.id, everything::NOT_RUNNING_RESULT_ID);
    assert!(stopped.item.display_name.contains("未运行"));
    assert!(everything_status_result(Availability::Ready).is_none());
}

#[test]
fn missing_dependency_is_always_visible_ahead_of_application_results() {
    let mut results = (0..crate::search::MAX_RESULTS)
        .map(|index| {
            let item = crate::model::AppItem::scanned(
                format!("app:{index}"),
                format!("Application {index}"),
                format!(r"C:\Apps\app-{index}.exe"),
                None,
                None,
                "start-menu",
            );
            crate::model::SearchResult::scored(item, 1000, "exact")
        })
        .collect::<Vec<_>>();
    let status = everything_status_result(Availability::NotInstalled).unwrap();

    prepend_dependency_status(&mut results, &[status]);

    assert_eq!(results[0].item.id, everything::DOWNLOAD_RESULT_ID);
    assert_eq!(results.len(), crate::search::MAX_RESULTS);
}

#[test]
fn direct_path_offers_two_actions_for_files_and_one_for_directories() {
    let dir = std::env::temp_dir().join(format!(
        "kite-path-input-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("extensionless & 100%");
    std::fs::write(&file, b"test").unwrap();

    let file_hits = build_direct_path_results(&file.to_string_lossy());
    assert_eq!(file_hits.len(), 2);
    assert_eq!(file_hits[0].item.display_name, "打开文件");
    assert!(matches!(
        file_hits[0].action,
        ResultAction::OpenLocalPath { .. }
    ));
    assert_eq!(file_hits[1].item.display_name, "打开文件路径");
    assert!(matches!(
        file_hits[1].action,
        ResultAction::RevealPath { .. }
    ));

    let folder_hits = build_direct_path_results(&dir.to_string_lossy());
    assert_eq!(folder_hits.len(), 1);
    assert_eq!(folder_hits[0].item.display_name, "打开文件路径");
    assert!(matches!(
        folder_hits[0].action,
        ResultAction::RevealPath { .. }
    ));

    std::fs::remove_file(&file).unwrap();
    assert!(build_direct_path_results(&file.to_string_lossy()).is_empty());
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn direct_path_actions_lead_and_replace_duplicate_file_hits() {
    let mut state = test_state(r"C:\Work\report.txt");
    let path = state.query.clone();
    let direct = vec![crate::model::SearchResult::scored(
        crate::model::AppItem::scanned(
            "direct:path".into(),
            "打开文件".into(),
            path.clone(),
            None,
            None,
            "direct-path",
        ),
        2000,
        "direct-path",
    )];
    let duplicate = crate::model::SearchResult::scored(
        crate::model::AppItem::scanned(
            "file:duplicate".into(),
            "report.txt".into(),
            path.to_uppercase(),
            None,
            None,
            "everything",
        ),
        400,
        "file",
    );
    let mut hits = vec![duplicate, super::super::test_support::settings_result()];
    prepend_direct_path_results(&mut hits, &direct);
    assert_eq!(hits[0].item.id, "direct:path");
    assert_eq!(hits.len(), 2);

    state.direct_path_results = direct.clone();
    state.apply_app_search_ready(
        state.app_query_generation,
        state.query.clone(),
        hits,
        0,
        state.index_generation,
    );
    assert_eq!(state.results[0].item.id, "direct:path");
    assert_eq!(state.results.len(), 2);

    state.direct_path_generation = 2;
    state.query = r"C:\Work\other.txt".into();
    state.direct_path_results.clear();
    let _ = super::super::interaction::update(
        &mut state,
        super::super::Message::DirectPathReady(1, r"C:\Work\report.txt".into(), direct),
    );
    assert!(state.direct_path_results.is_empty());
}

#[test]
fn empty_query_lists_put_recent_before_pinned_fill() {
    use crate::model::AppItem;

    let app = |id: &str| {
        let mut item = AppItem::scanned(
            id.into(),
            id.into(),
            format!(r"C:\{id}.exe"),
            None,
            None,
            "start-menu",
        );
        item.attach_search_fields();
        item
    };
    let apps = vec![app("a"), app("b"), app("c")];
    let pinned = vec!["c".to_string()];

    let lists = build_empty_query_lists(&["b".into(), "c".into()], &pinned, &apps);
    assert!(
        lists.recent.iter().all(|h| h.item.id != "c"),
        "固定项不得重复出现在最近区"
    );
    assert_eq!(lists.pinned.len(), 1);
    assert_eq!(lists.pinned[0].item.id, "c");
    assert!(lists.recent.iter().any(|h| h.item.id == "b"));
}

#[test]
fn empty_query_pinned_follow_storage_order_and_cap() {
    use crate::model::AppItem;

    let app = |id: &str| {
        let mut item = AppItem::scanned(
            id.into(),
            id.into(),
            format!(r"C:\{id}.exe"),
            None,
            None,
            "start-menu",
        );
        item.attach_search_fields();
        item
    };
    // 10 个可固定应用；pinned_ids 模拟存储层 pinned_at DESC
    let apps: Vec<_> = (0..10).map(|i| app(&format!("p{i}"))).collect();
    let pinned: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();

    let lists = build_empty_query_lists(&[], &pinned, &apps);
    assert_eq!(lists.pinned.len(), 8, "固定项最多 8");
    assert_eq!(
        lists.pinned[0].item.id, "p0",
        "必须按 pinned_ids 给定顺序截断，而非 HashSet 迭代序"
    );
    assert_eq!(lists.pinned[7].item.id, "p7");
    assert!(
        lists.pinned.iter().all(|h| h.item.id != "p8" && h.item.id != "p9"),
        "超出上限的固定项不得因哈希序挤入选中集合"
    );
}

#[test]
fn stale_results_cannot_be_launched_until_current_query_lands() {
    use crate::ui::actions::launch_selected;

    // 结果已就绪：可启动
    let mut state = test_state("k");
    let _ = launch_selected(&mut state);
    assert!(state.settings_open, "就绪列表应可启动");

    // 提交新查询、结果尚未返回：列表保留旧内容，但启动必须被拒绝
    let mut state = test_state("k");
    state.query = "chrome".into();
    state.refresh_results();
    assert_eq!(state.results.len(), 1, "旧列表应继续显示，不闪空");
    let _ = launch_selected(&mut state);
    assert!(
        !state.settings_open,
        "新查询结果未返回前不得启动旧列表里的行"
    );

    // 结果落地后恢复可启动
    let generation = state.app_query_generation;
    let index_generation = state.index_generation;
    state.apply_app_search_ready(
        generation,
        "chrome".into(),
        vec![settings_result()],
        0,
        index_generation,
    );
    assert_eq!(
        state.results.first().map(|r| r.item.id.as_str()),
        Some("kite:settings"),
        "落地后第 0 行应仍是可启动的内置项"
    );
    let _ = launch_selected(&mut state);
    assert!(state.settings_open, "新结果落地后应恢复启动资格");
}

#[test]
fn alias_change_invalidates_base_cache_and_resubmits() {
    use crate::ui::actions::refresh_after_alias_change;

    let mut state = test_state("k");
    let epoch = state.base_hit_cache.epoch();
    state
        .base_hit_cache
        .insert_if_epoch(epoch, 0, "k", Vec::new());
    assert!(!state.base_hit_cache.is_empty(), "先让缓存里有一条");

    let before = state.app_search_worker.latest_seq();
    refresh_after_alias_change(&mut state);

    assert!(
        state.base_hit_cache.is_empty(),
        "别名变化必须清空基础候选缓存"
    );
    assert!(
        state.app_search_worker.latest_seq() > before,
        "别名变化要与逐键输入走同一条刷新入口（在途请求由此作废）"
    );
}

/// 关闭查询日志后，`qlog` 必须直接返回：一旦它仍然构造消息，`unreachable!` 就会 panic。
/// 不构造消息 ⇒ 不调用 `plog` ⇒ 按键路径上没有同步文件写入。
/// 按键频率上的日志（Alt 按下/抬起、IME 组合/提交、查询刷新）共用这一个闸门。
#[test]
fn query_log_off_never_builds_the_message() {
    let mut state = test_state("k");
    state.query_log = false;

    state.qlog(|| unreachable!("关闭查询日志后不得构造日志消息"));
}

/// 开关只影响日志：关掉之后输入事件处理本身照旧。
#[test]
fn query_log_off_leaves_input_handlers_working() {
    use crate::ui::interaction::update;

    let mut state = test_state("");
    state.query_log = false;

    let _ = update(&mut state, crate::ui::Message::Composing(true));
    assert!(state.ime_composing, "关掉日志不得影响 IME 组合态");
    let _ = update(&mut state, crate::ui::Message::Composing(false));
    assert!(!state.ime_composing);
    let _ = update(&mut state, crate::ui::Message::ImeCommit("你好".into()));
}

/// 默认保留可诊断性：开关开启时查询日志照常构造并写出。
#[test]
fn query_log_on_still_emits_by_default() {
    let state = test_state("k");
    assert!(state.query_log, "查询日志必须默认开启");

    let built = std::cell::Cell::new(0);
    state.qlog(|| {
        built.set(built.get() + 1);
        "app search ready generation=1".to_string()
    });

    assert_eq!(built.get(), 1, "开启时查询日志必须照常发射");
}

/// 开关只是日志闸门：同一查询在开／关两种状态下，结果顺序、代际与缓存完全一致。
#[test]
fn query_log_switch_does_not_change_results_or_cache() {
    let run = |query_log: bool| {
        let mut state = test_state("k");
        state.query_log = query_log;
        // 先让缓存里有内容，确认开关既不写入也不清空缓存。
        let epoch = state.base_hit_cache.epoch();
        state
            .base_hit_cache
            .insert_if_epoch(epoch, 0, "k", Vec::new());

        state.refresh_results();
        let generation = state.app_query_generation;
        let index_generation = state.index_generation;
        state.apply_app_search_ready(
            generation,
            "k".into(),
            vec![settings_result()],
            0,
            index_generation,
        );
        state
    };

    let on = run(true);
    let off = run(false);

    let ids = |s: &crate::ui::State| {
        s.results
            .iter()
            .map(|r| (r.item.id.clone(), r.score, r.matched_by.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&on), ids(&off), "开关不得改变结果内容与顺序");
    assert_eq!(
        on.app_query_generation, off.app_query_generation,
        "开关不得改变查询代际"
    );
    assert_eq!(on.results_stale, off.results_stale);
    assert_eq!(
        on.base_hit_cache.epoch(),
        off.base_hit_cache.epoch(),
        "开关不得推动缓存代际"
    );
    assert_eq!(on.base_hit_cache.is_empty(), off.base_hit_cache.is_empty());
    assert!(!off.base_hit_cache.is_empty(), "关掉日志不得顺手清空缓存");
}

/// 切换开关的消息只落盘：不重跑搜索、不定代际、不动结果与缓存。
#[test]
fn set_query_log_toggle_leaves_search_state_untouched() {
    use crate::ui::interaction::update;

    let mut state = test_state("k");
    state.refresh_results();
    let generation = state.app_query_generation;
    let index_generation = state.index_generation;
    let submitted = state.app_search_worker.latest_seq();
    let stale = state.results_stale;
    let epoch = state.base_hit_cache.epoch();
    let results = state.results.len();

    let _ = update(&mut state, crate::ui::Message::SetQueryLog(false));

    assert!(!state.query_log, "开关必须真的落到位");
    assert_eq!(state.app_query_generation, generation);
    assert_eq!(state.index_generation, index_generation);
    assert_eq!(state.results_stale, stale);
    assert_eq!(state.results.len(), results);
    assert_eq!(state.base_hit_cache.epoch(), epoch, "不得失效缓存");
    assert_eq!(
        state.app_search_worker.latest_seq(),
        submitted,
        "切换日志开关不得重新提交搜索"
    );
}
