//! 测试专用状态构造。跨 `ui` 子模块共用，避免每处各写一份 `State` 字面量
//! （`State` 增字段时只需改这里）。

use super::*;

/// 一条内置「Kite 设置」结果，作为可启动的目标。
pub(super) fn settings_result() -> SearchResult {
    SearchResult::scored(
        AppItem::scanned(
            "kite:settings".into(),
            "Kite 设置".into(),
            "kite:settings".into(),
            None,
            None,
            "builtin",
        ),
        930,
        "builtin",
    )
}

/// 空索引、无历史库的最小状态，结果预置 [`settings_result`]。
pub(super) fn test_state(query: &str) -> State {
    State {
        data_dir: std::env::temp_dir(),
        icon_dir: std::env::temp_dir(),
        index: std::sync::Arc::new(Mutex::new(AppIndex::empty())),
        scan_options: std::sync::Arc::new(std::sync::RwLock::new(
            app::scanner::ScanOptions::default(),
        )),
        history: None,
        input_id: search_view::input_id(),
        window_id: None,
        query: query.into(),
        results: vec![settings_result()],
        selected: 0,
        hidden: false,
        ime_composing: false,
        alt_down: false,
        query_at_alt: None,
        alt_digit_consumed: false,
        index_ready: true,
        rescan_pending: false,
        files_mode: false,
        file_query_generation: 0,
        file_results: Vec::new(),
        app_query_generation: 0,
        results_stale: false,
        index_generation: 0,
        base_hit_cache: std::sync::Arc::new(search::service::BaseHitCache::default()),
        app_search_worker: std::sync::Arc::new(search::service::AppSearchWorker::spawn()),
        menu: None,
        pinned: Default::default(),
        cursor: Default::default(),
        settings_open: false,
        settings_section: Section::General,
        hide_on_blur: true,
        autostart: false,
        history_recording: true,
        query_log: true,
        search_engine: "auto".into(),
        search_engine_custom: String::new(),
        web_search_hotkey: system::hotkey::DEFAULT_WEB_SEARCH_HOTKEY.into(),
        hotkey: "Alt+Space".into(),
        hotkey_label: "Alt+Space".into(),
        aliases: Vec::new(),
        alias_input: String::new(),
        alias_target_input: String::new(),
        alias_candidates: Vec::new(),
        alias_pick: None,
        portable_dirs: Vec::new(),
        portable_dir_input: String::new(),
        flash: None,
        hotkey_recording: false,
        update_status: None,
        update_asset: None,
        update_checking: false,
        epoch: 0,
        hover_suppressed: false,
        last_hover_pt: None,
    }
}
