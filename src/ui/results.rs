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
        std::thread::spawn(move || {
            let started = Instant::now();
            let results = build_file_results(&query, &icon_dir);
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

        let retrieval = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            index.retrieval.clone()
        };
        let (apps, system_entries) = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            (index.apps.clone(), index.system_entries.clone())
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
            retrieval,
            apps,
            system_entries,
            cache,
            cache_epoch,
            on_done: Arc::new(move |generation, query, hits, elapsed_us| {
                let _ = EVENT_TX
                    .get()
                    .expect("event tx")
                    .unbounded_send(Message::AppSearchReady(
                        generation,
                        query,
                        hits,
                        elapsed_us,
                        index_gen,
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
            plog(&format!(
                "app search stale generation={generation} current={} index_gen={index_generation} cur_index={} query={query:?}",
                self.app_query_generation, self.index_generation
            ));
            return;
        }
        let q_norm = search::normalize_for_index(&self.query);
        let mut hits = self.merge_aux_hits(hits, &q_norm);
        if self.files_mode {
            prepend_dependency_status(&mut hits, &self.file_results);
        }
        self.results = hits;
        self.selected = 0;
        self.hover_suppressed = false;
        // 结果落地才恢复「可启动」资格：Enter / Alt+数字 / 点击共用这一个判定。
        self.results_stale = false;
        let top = self
            .results
            .first()
            .map(|r| r.item.display_name.as_str())
            .unwrap_or("-");
        plog(&format!(
            "app search ready generation={generation} query={query:?} -> {} results in {elapsed_us}us top='{top}' epoch={}",
            self.results.len(),
            self.epoch
        ));
    }

    /// 最终列表组装的唯一实现：应用命中 → 网址直达 → 合并 Everything 文件 →
    /// 截断 → 网页搜索槽位 → 嗅探搜索引擎模板写回。
    ///
    /// 调用方只有异步主路径（`apply_app_search_ready`）；同步刷新不再内联一份，
    /// 避免「第 5 位网页搜索」「文件结果过滤」这类规则改一处漏一处。
    fn merge_aux_hits(&mut self, mut hits: Vec<SearchResult>, q_norm: &str) -> Vec<SearchResult> {
        let preferred = self.history.as_ref().and_then(|h| h.preferred_browser());
        let search_template = self.history.as_ref().and_then(|h| h.search_url_template());
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
    fn cache_detected_search_template(&mut self, missing: bool, preferred: Option<&str>) {
        if !missing {
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
    /// - 空 Query：固定 + 最近（同步；不需要应用召回与链接/文件/网页槽）；
    /// - 非空 Query：交给常驻 worker，由 `apply_app_search_ready` → `merge_aux_hits`
    ///   组装并落地（含合并、选中复位与日志）。
    ///
    /// 非空分支不再内联搜索：Pipeline 只此一条，Pin/降权/文件切换等同步动作
    /// 与逐键输入走同一条路径。
    pub(super) fn refresh_results(&mut self) {
        let t0 = Instant::now();
        let q_norm = search::normalize_for_index(&self.query);
        if let Some(db) = &self.history {
            self.pinned = db.pinned_ids().into_iter().collect();
        }
        if !q_norm.is_empty() {
            self.request_app_search();
            return;
        }

        let (recent, pinned) = self
            .history
            .as_ref()
            .map(|h| {
                (
                    h.recent_ids(search::MAX_RESULTS).unwrap_or_default(),
                    h.pinned_ids(),
                )
            })
            .unwrap_or_default();
        let default_list = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            search::order_by_recent(&index.apps, &recent, &pinned, search::MAX_RESULTS)
        };
        self.results = default_list;
        if self.files_mode {
            prepend_dependency_status(&mut self.results, &self.file_results);
        }
        self.selected = 0;
        // 列表刷新后回到顶部，避免选中行与滚动位置错位。
        self.hover_suppressed = false;
        // 空 Query 的同步列表就是当前状态，不经过后台，直接恢复可启动资格。
        self.results_stale = false;
        let top = self
            .results
            .first()
            .map(|r| r.item.display_name.as_str())
            .unwrap_or("-");
        plog(&format!(
            "query '{:?}' -> {} results in {}us top='{top}' epoch={}",
            self.query,
            self.results.len(),
            t0.elapsed().as_micros(),
            self.epoch
        ));
    }
}

fn build_file_results(query: &str, icon_dir: &std::path::Path) -> Vec<SearchResult> {
    if let Some(status) = everything_status_result(system::everything::availability()) {
        return vec![status];
    }
    if search::normalize_for_index(query).chars().count() < 2 {
        return Vec::new();
    }
    system::everything::search_files(query, 20)
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

        assert_eq!(
            state.app_search_worker.latest_seq(),
            0,
            "空 Query 走固定+最近，不应提交 worker"
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
    fn missing_everything_is_visible_as_a_search_result() {
        if everything::availability() != Availability::NotInstalled {
            return;
        }

        let results = build_file_results("logo", &std::env::temp_dir());

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
        state.base_hit_cache.insert_if_epoch(epoch, 0, "k", Vec::new());
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
}
