//! 搜索结果刷新与已有结果源合并。

use super::*;
use std::sync::Arc;

impl State {
    /// 使正在运行的文件查询失效，并清除其结果。
    pub(super) fn invalidate_file_search(&mut self) {
        self.file_query_generation = self.file_query_generation.wrapping_add(1);
        self.file_results.clear();
    }

    /// 探测依赖状态，并在可用时把 Everything 查询放到后台线程。
    pub(super) fn request_file_search(&mut self) {
        self.invalidate_file_search();
        let query = self.query.trim().to_string();
        if !self.files_mode {
            return;
        }

        let generation = self.file_query_generation;
        let icon_dir = self.icon_dir.clone();
        let filter = self.file_filter;
        std::thread::spawn(move || {
            let started = Instant::now();
            let results = build_file_results(&query, &icon_dir, filter);
            let elapsed_us = started.elapsed().as_micros();
            let _ = EVENT_TX
                .get()
                .expect("event tx")
                .unbounded_send(Message::FileSearchReady(
                    generation, query, results, elapsed_us,
                ));
        });
    }

    /// 读取当前个性化快照（历史/Pin/降权），供后台 worker 使用。
    pub(super) fn snapshot_personalization(
        &self,
        q_norm: &str,
    ) -> Option<history::Personalization> {
        self.history.as_ref().map(|db| history::Personalization {
            usage: db.usage_all(),
            pairs: db.query_pairs_for(q_norm),
            pinned: db.pinned_ids().into_iter().collect(),
            demoted: db.demoted_ids().into_iter().collect(),
            now: storage::now_ts(),
            query_norm: q_norm.to_string(),
        })
    }

    /// 已展示列表是否属于当前查询（即是否具备「可启动」资格）。
    /// 空 Query 的同步列表总是当前；新查询提交后、结果落地前为假。
    pub(super) fn results_are_launchable(&self) -> bool {
        !self.results_stale
    }

    /// 提交常驻 worker：最新请求覆盖；个性化在截断前进入统一排序。
    /// 缓存存的是个性化前候选，个性化每次按最新偏好重放。
    ///
    /// 提交后旧列表继续显示但立即失去启动资格（`results_stale`），
    /// 直到本条结果由 `apply_app_search_ready` 落地。
    pub(super) fn request_app_search(&mut self) {
        self.app_query_generation = self.app_query_generation.wrapping_add(1);
        let generation = self.app_query_generation;
        let query = self.query.clone();
        let q_norm = search::normalize_for_index(&query);
        if q_norm.is_empty() {
            return;
        }
        self.results_stale = true;

        // 一次加锁取到自洽的一份来源：有预建索引就按引用共享，
        // 绝不在这条按键路径上复制整份应用数组与系统入口数组。
        let source = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            match index.retrieval.clone() {
                Some(retrieval) => search::service::IndexSource::Prebuilt(retrieval),
                None => search::service::IndexSource::Snapshot {
                    apps: index.apps.clone(),
                    system_entries: index.system_entries.clone(),
                },
            }
        };
        let user_targets: Vec<search::UserTarget> = self
            .history
            .as_ref()
            .map(|h| {
                h.alias_matches(&q_norm)
                    .into_iter()
                    .map(|a| search::UserTarget {
                        id: a.target_id,
                        name: a.target_name.to_lowercase(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let prefs = self.snapshot_personalization(&q_norm);
        let cache = self.base_hit_cache.clone();
        let cache_epoch = cache.epoch();
        let index_gen = self.index_generation;

        let job = search::service::AppSearchJob {
            generation,
            index_generation: index_gen,
            query,
            q_norm,
            user_targets,
            prefs,
            source,
            cache,
            cache_epoch,
            on_done: Arc::new(move |generation, query, hits, elapsed_us| {
                let _ = EVENT_TX
                    .get()
                    .expect("event tx")
                    .unbounded_send(Message::AppSearchReady(
                        generation, query, hits, elapsed_us, index_gen,
                    ));
            }),
        };
        self.app_search_worker.submit(job);
    }

    /// 应用后台结果：Query/索引代际核对后合并链接/文件/网页槽位。
    /// 非空 Query 的最终列表全部由此落地（`refresh_results` 不再内联一份）。
    pub(super) fn apply_app_search_ready(
        &mut self,
        generation: u64,
        query: String,
        hits: Vec<SearchResult>,
        elapsed_us: u128,
        index_generation: u64,
    ) {
        if generation != self.app_query_generation
            || query != self.query
            || index_generation != self.index_generation
        {
            self.qlog(|| {
                format!(
                    "app search stale generation={generation} current={} index_gen={index_generation} cur_index={} query={query:?}",
                    self.app_query_generation, self.index_generation
                )
            });
            return;
        }
        let q_norm = search::normalize_for_index(&self.query);
        let mut hits = self.merge_aux_hits(hits, &q_norm);
        // Command 静态入口：不启动插件进程；按分数插入，不无条件顶掉本机精确命中
        {
            let reg = self
                .plugin_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let commands = plugin::command_hits(&reg, self.query.trim());
            for cmd in commands.into_iter() {
                if hits.iter().any(|h| h.item.id == cmd.item.id) {
                    continue;
                }
                let pos = hits
                    .iter()
                    .position(|h| h.score < cmd.score)
                    .unwrap_or(hits.len());
                hits.insert(pos, cmd);
            }
        }
        if self.files_mode {
            prepend_dependency_status(&mut hits, &self.file_results);
        }
        self.results = hits;
        self.selected = 0;
        self.navigation_mode = NavigationMode::Input;
        self.hover_suppressed = false;
        // 结果落地才恢复「可启动」资格：Enter / Alt+数字 / 点击共用这一个判定。
        self.results_stale = false;
        let top = self
            .results
            .first()
            .map(|r| r.item.display_name.as_str())
            .unwrap_or("-");
        self.qlog(|| {
            format!(
                "app search ready generation={generation} query={query:?} -> {} results in {elapsed_us}us top='{top}' epoch={}",
                self.results.len(),
                self.epoch
            )
        });
    }

    /// 最终列表组装的唯一实现：应用命中 → 网址直达 → 合并 Everything 文件 →
    /// 截断 → 网页搜索槽位 → 嗅探搜索引擎模板写回。
    ///
    /// 调用方只有异步主路径（`apply_app_search_ready`）；同步刷新不再内联一份，
    /// 避免「第 5 位网页搜索」「文件结果过滤」这类规则改一处漏一处。
    fn merge_aux_hits(&mut self, mut hits: Vec<SearchResult>, q_norm: &str) -> Vec<SearchResult> {
        let preferred = self.history.as_ref().and_then(|h| h.preferred_browser());
        let search_template = if self.search_engine == "auto" {
            self.history.as_ref().and_then(|h| h.search_url_template())
        } else {
            crate::system::search_engine::preset_by_id(&self.search_engine)
                .map(|p| p.template)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    // custom：用编辑框内容
                    crate::system::search_engine::is_valid_template(&self.search_engine_custom)
                        .then(|| self.search_engine_custom.trim().to_string())
                })
                .or_else(|| self.history.as_ref().and_then(|h| h.search_url_template()))
        };
        let is_url = search::url::normalize_url(&self.query).is_some();
        if let Some(url) = search::url::normalize_url(&self.query) {
            let mut merged = app::web::build_hits(&url, preferred.as_deref(), &self.icon_dir);
            merged.append(&mut hits);
            hits = merged;
        }
        if self.files_mode {
            hits.extend(
                self.file_results
                    .iter()
                    .filter(|result| result.item.source != "everything-status")
                    .cloned(),
            );
        }
        hits.truncate(search::MAX_RESULTS);
        if !is_url && !q_norm.is_empty() {
            let has_app_like = hits
                .iter()
                .any(|h| h.item.source != "browser" && h.item.source != "websearch");
            if has_app_like {
                if let Some(web) = app::web::build_primary_search_hit(
                    self.query.trim(),
                    preferred.as_deref(),
                    &self.icon_dir,
                    search_template.as_deref(),
                ) {
                    hits = app::web::insert_at_slot(hits, web, app::web::WEB_SEARCH_SLOT);
                }
            } else {
                hits = app::web::build_search_hits(
                    self.query.trim(),
                    preferred.as_deref(),
                    &self.icon_dir,
                    search_template.as_deref(),
                );
                hits = search::rerank(hits, search::MAX_RESULTS);
            }
        }
        self.cache_detected_search_template(search_template.is_none(), preferred.as_deref());
        hits
    }

    /// 未缓存模板时嗅探一次并写回（副本库），避免每次读浏览器配置。
    /// 用户已选固定引擎（非 auto）时不覆盖。
    fn cache_detected_search_template(&mut self, missing: bool, preferred: Option<&str>) {
        if !missing || self.search_engine != "auto" {
            return;
        }
        let Some(pref) = preferred else {
            return;
        };
        let Some(template) = system::search_engine::detect_search_template(pref) else {
            return;
        };
        if let Some(db) = self.history.as_mut() {
            let _ = db.set_search_url_template(&template);
        }
    }

    /// 结果刷新唯一入口：
    /// - Plugin Provider Mode：Trigger 命中时整列表交给插件，不与 Core 混排；
    /// - 空 Query：固定 + 最近（同步）；
    /// - 非空 Query：worker + Command 静态入口合并。
    pub(super) fn refresh_results(&mut self) {
        let t0 = Instant::now();
        if let Some(db) = &self.history {
            self.pinned = db.pinned_ids().into_iter().collect();
        }

        // Provider Mode：显式 Trigger，整查询交给插件
        let activation = {
            let reg = self
                .plugin_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            plugin::route_query(&self.query, &reg)
        };
        if let Some(act) = activation {
            // 非内联工具（JSON 独立窗）：不进 Provider；由 UI 二次确认后开窗。
            if act.plugin_id == "com.kite.devtools" && act.provider_id == "json" {
                if self.provider_mode.is_some() {
                    self.exit_provider_mode();
                }
            } else {
                self.results_stale = true;
                self.plugin_panel = None;
                self.plugin_flash = None;
                self.provider_mode = Some(act.clone());
                self.plugin_query_generation = self.plugin_query_generation.wrapping_add(1);
                let gen = self.plugin_query_generation;
                let registry = self.plugin_registry.clone();
                let host = self.plugin_host.clone();
                let act_for_thread = act.clone();
                std::thread::spawn(move || {
                    let (payload, host_calls) = {
                        let reg = registry.lock().unwrap_or_else(|e| e.into_inner());
                        let mut host = host.lock().unwrap_or_else(|e| e.into_inner());
                        host.set_generation(gen);
                        host.idle_sweep();
                        let payload = match host.query(&reg, &act_for_thread, gen) {
                            Ok(plugin::QueryOutcome::List {
                                plugin_id,
                                provider_id,
                                items,
                                ..
                            }) => {
                                let mapped = items
                                    .iter()
                                    .map(|it| {
                                        plugin::host::list_item_to_search_result(
                                            &plugin_id,
                                            &provider_id,
                                            it,
                                        )
                                    })
                                    .collect();
                                PluginQueryPayload::List {
                                    plugin_id,
                                    provider_id,
                                    items: mapped,
                                }
                            }
                            Ok(plugin::QueryOutcome::Panel {
                                plugin_id,
                                provider_id,
                                panel,
                                ..
                            }) => PluginQueryPayload::Panel {
                                plugin_id,
                                provider_id,
                                panel,
                            },
                            Ok(plugin::QueryOutcome::Empty {
                                plugin_id,
                                provider_id,
                                ..
                            }) => PluginQueryPayload::Empty {
                                plugin_id,
                                provider_id,
                            },
                            // 过期代际不是插件故障：静默丢弃，避免 UI 误标「崩溃」。
                            Err(plugin::HostError::StaleGeneration { .. }) => {
                                PluginQueryPayload::Empty {
                                    plugin_id: act_for_thread.plugin_id.clone(),
                                    provider_id: act_for_thread.provider_id.clone(),
                                }
                            }
                            Err(e) => PluginQueryPayload::Error(e.to_string()),
                        };
                        let host_calls = host.drain_host_calls();
                        (payload, host_calls)
                    };
                    if let Some(tx) = EVENT_TX.get() {
                        for (pid, call) in host_calls {
                            let _ = tx.unbounded_send(Message::PluginHostCall(pid, call));
                        }
                        let _ = tx.unbounded_send(Message::PluginQueryReady(gen, payload));
                    }
                });
                self.qlog(|| format!("provider mode enter {:?}", act.provider_id));
                return;
            }
        }

        // Trigger 不再命中：退出 Provider Mode，回到 Core Search
        if self.provider_mode.is_some() {
            self.exit_provider_mode();
        }

        let q_norm = search::normalize_for_index(&self.query);
        if !q_norm.is_empty() {
            self.request_app_search();
            return;
        }

        self.app_search_worker.cancel_current();

        let (recent_ids, pinned_ids) = self
            .history
            .as_ref()
            .map(|h| {
                (
                    h.recent_ids(search::MAX_RESULTS).unwrap_or_default(),
                    h.pinned_ids(),
                )
            })
            .unwrap_or_default();

        let index_apps = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            index.apps.clone()
        };

        // 1. 最近使用应用（真实启动历史，排除已固定的）
        let mut recent_items: Vec<SearchResult> = Vec::new();
        for id in &recent_ids {
            if pinned_ids.contains(id) {
                continue;
            }
            if recent_items.iter().any(|h| &h.item.id == id) {
                continue;
            }
            if let Some(app) = index_apps.iter().find(|a| &a.id == id) {
                recent_items.push(SearchResult::scored(app.clone(), 1, "recent"));
            }
            if recent_items.len() >= 16 {
                break;
            }
        }

        // 2. 补满至 16 项（排除已固定、已在列表中与隐藏项）
        for app in &index_apps {
            if recent_items.len() >= 16 {
                break;
            }
            if pinned_ids.contains(&app.id) || recent_items.iter().any(|h| h.item.id == app.id) {
                continue;
            }
            if crate::model::is_hidden_on_empty_fill(&app.source, &app.target) {
                continue;
            }
            recent_items.push(SearchResult::scored(app.clone(), 0, "default"));
        }

        // 3. 已固定项目（最多 8 项）
        let mut pinned_items: Vec<SearchResult> = Vec::new();
        for id in &pinned_ids {
            if let Some(app) = index_apps.iter().find(|a| &a.id == id) {
                pinned_items.push(SearchResult::scored(app.clone(), 2, "pinned"));
            }
            if pinned_items.len() >= 8 {
                break;
            }
        }

        self.grid_recent_count = recent_items.len();
        let mut results = recent_items;
        results.extend(pinned_items);
        self.results = results;
        if self.files_mode {
            prepend_dependency_status(&mut self.results, &self.file_results);
        }
        self.selected = 0;
        self.navigation_mode = NavigationMode::Input;
        self.hover_suppressed = false;
        self.results_stale = false;
        let top = self
            .results
            .first()
            .map(|r| r.item.display_name.as_str())
            .unwrap_or("-");
        self.qlog(|| {
            format!(
                "query '{:?}' -> {} results in {}us top='{top}' epoch={}",
                self.query,
                self.results.len(),
                t0.elapsed().as_micros(),
                self.epoch
            )
        });
    }

    pub(super) fn exit_provider_mode(&mut self) {
        self.provider_mode = None;
        self.plugin_panel = None;
        self.plugin_query_generation = self.plugin_query_generation.wrapping_add(1);
        self.results_stale = false;
    }

    /// Provider Mode 查询落地。
    pub(super) fn apply_plugin_query_ready(
        &mut self,
        generation: u64,
        payload: PluginQueryPayload,
    ) {
        if generation != self.plugin_query_generation || self.provider_mode.is_none() {
            return;
        }
        match payload {
            PluginQueryPayload::List {
                plugin_id,
                provider_id,
                items,
                ..
            } => {
                self.qlog(|| {
                    format!(
                        "plugin list ready {plugin_id}/{provider_id} n={}",
                        items.len()
                    )
                });
                let mut items = items;
                // priority 仅影响当前 Provider 内部顺序，不影响 Core Ranking。
                items.sort_by(|a, b| {
                    b.score
                        .cmp(&a.score)
                        .then_with(|| a.item.id.cmp(&b.item.id))
                });
                self.results = items;
                self.selected = 0;
                self.navigation_mode = NavigationMode::Input;
                self.results_stale = false;
                self.plugin_panel = None;
                self.plugin_flash = None;
            }
            PluginQueryPayload::Panel {
                plugin_id,
                provider_id,
                panel,
                ..
            } => {
                self.qlog(|| format!("plugin panel ready {plugin_id}/{provider_id}"));
                self.plugin_panel = Some(panel);
                self.results = Vec::new();
                self.selected = 0;
                self.navigation_mode = NavigationMode::Input;
                self.results_stale = false;
                self.plugin_flash = None;
            }
            PluginQueryPayload::Empty {
                plugin_id,
                provider_id,
                ..
            } => {
                self.qlog(|| format!("plugin empty ready {plugin_id}/{provider_id}"));
                self.results = Vec::new();
                self.plugin_panel = None;
                self.results_stale = false;
                self.plugin_flash = None;
            }
            PluginQueryPayload::Error(err) => {
                self.plugin_flash = Some(err);
                self.results = Vec::new();
                self.plugin_panel = None;
                self.results_stale = false;
            }
        }
    }
}

fn build_file_results(
    query: &str,
    icon_dir: &std::path::Path,
    filter: system::everything::FileFilter,
) -> Vec<SearchResult> {
    if let Some(status) = everything_status_result(system::everything::availability()) {
        return vec![status];
    }
    if search::normalize_for_index(query).chars().count() < 2 {
        return Vec::new();
    }
    system::everything::search_files(query, 20, filter)
        .into_iter()
        .map(|hit| {
            let id = format!("file:{}", hit.path.to_lowercase());
            let mut item =
                AppItem::scanned(id, hit.name.clone(), hit.path, None, None, "everything");
            item.attach_search_fields();
            item.icon = system::icons::cache_type_icon(icon_dir, &hit.name, hit.is_folder);
            SearchResult::scored(item, 400, "file")
        })
        .collect()
}

fn everything_status_result(
    availability: system::everything::Availability,
) -> Option<SearchResult> {
    use system::everything::{
        Availability, DOWNLOAD_RESULT_ID, DOWNLOAD_URL, NOT_RUNNING_RESULT_ID,
    };

    let (id, name, target) = match availability {
        Availability::Ready => return None,
        Availability::InstalledButNotRunning => (
            NOT_RUNNING_RESULT_ID,
            "Everything 未运行，文件搜索不可用",
            NOT_RUNNING_RESULT_ID,
        ),
        Availability::NotInstalled => (
            DOWNLOAD_RESULT_ID,
            "未安装 Everything，点击前往官方下载",
            DOWNLOAD_URL,
        ),
    };
    let mut item = AppItem::scanned(
        id.to_string(),
        name.to_string(),
        target.to_string(),
        None,
        None,
        "everything-status",
    );
    item.attach_search_fields();
    Some(SearchResult::scored(item, 0, "everything-status"))
}

fn prepend_dependency_status(results: &mut Vec<SearchResult>, file_results: &[SearchResult]) {
    if let Some(status) = file_results
        .iter()
        .find(|result| result.item.source == "everything-status")
    {
        results.insert(0, status.clone());
        results.truncate(search::MAX_RESULTS);
    }
}

pub(super) fn is_current_file_response(
    files_mode: bool,
    current_generation: u64,
    current_query: &str,
    response_generation: u64,
    response_query: &str,
) -> bool {
    files_mode
        && current_generation == response_generation
        && current_query.trim() == response_query
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{settings_result, test_state};
    use super::{
        build_file_results, everything_status_result, is_current_file_response,
        prepend_dependency_status,
    };
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
    fn changing_file_filter_invalidates_previous_query() {
        use super::super::interaction::update;
        use super::super::Message;

        let mut state = test_state("Kite");
        state.files_mode = true;
        state.file_query_generation = 4;
        let _ = update(&mut state, Message::FileFilterChanged(everything::FileFilter::Images));

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

        let stopped = everything_status_result(Availability::InstalledButNotRunning)
            .expect("not-running status");
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
}
