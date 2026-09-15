//! 搜索结果刷新与已有结果源合并。

use super::*;

/// 后台执行一次应用检索（含个性化，截断前）。
fn run_app_search(
    retrieval: Option<&search::RetrievalIndex>,
    apps: &[AppItem],
    system_entries: &[AppItem],
    query: &str,
    user_targets: &[search::UserTarget],
    prefs: Option<&history::Personalization>,
) -> Vec<SearchResult> {
    if let Some(ret) = retrieval {
        search::search_with_personalization(
            ret,
            query,
            user_targets,
            prefs,
            search::MAX_RESULTS,
        )
    } else {
        search::search_system_personalized(
            apps,
            system_entries,
            query,
            user_targets,
            prefs,
            search::MAX_RESULTS,
        )
    }
}

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

    /// 空 Query 同步默认列表；非空走后台应用搜索。
    pub(super) fn refresh_search_for_query(&mut self) {
        if search::normalize_for_index(&self.query).is_empty() {
            self.refresh_results();
        } else {
            self.request_app_search();
        }
    }

    /// 提交后台应用搜索：最新 Query 优先；个性化在截断前进入统一排序。
    /// 仅当无个性化状态时缓存基础命中，避免「先截断再加分」丢候选。
    pub(super) fn request_app_search(&mut self) {
        self.app_query_generation = self.app_query_generation.wrapping_add(1);
        let generation = self.app_query_generation;
        let query = self.query.clone();
        let q_norm = search::normalize_for_index(&query);
        if q_norm.is_empty() {
            return;
        }

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
        let cacheable = search::service::prefs_is_empty(prefs.as_ref());
        let cache = self.base_hit_cache.clone();
        let index_gen = self.index_generation;

        std::thread::spawn(move || {
            let started = Instant::now();
            let hits = if cacheable {
                let cached = cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(index_gen, &q_norm, search::MAX_RESULTS);
                if let Some(hits) = cached {
                    hits
                } else {
                    let hits = run_app_search(
                        retrieval.as_deref(),
                        &apps,
                        &system_entries,
                        &query,
                        &user_targets,
                        prefs.as_ref(),
                    );
                    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(
                        index_gen,
                        &q_norm,
                        search::MAX_RESULTS,
                        hits.clone(),
                    );
                    hits
                }
            } else {
                // 有历史/Pin/降权：个性化必须在截断前生效，不走截断缓存
                run_app_search(
                    retrieval.as_deref(),
                    &apps,
                    &system_entries,
                    &query,
                    &user_targets,
                    prefs.as_ref(),
                )
            };
            let elapsed_us = started.elapsed().as_micros();
            let _ = EVENT_TX
                .get()
                .expect("event tx")
                .unbounded_send(Message::AppSearchReady(
                    generation, query, hits, elapsed_us,
                ));
        });
    }

    /// 应用后台结果：代际核对后合并链接/文件/网页槽位（个性化已在 worker 完成）。
    pub(super) fn apply_app_search_ready(
        &mut self,
        generation: u64,
        query: String,
        hits: Vec<SearchResult>,
        elapsed_us: u128,
    ) {
        if generation != self.app_query_generation || query != self.query {
            plog(&format!(
                "app search stale generation={generation} current={} query={query:?}",
                self.app_query_generation
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

    /// 网址 / Everything 文件 / 网页搜索槽位合并（与同步路径同一套规则）。
    fn merge_aux_hits(&self, mut hits: Vec<SearchResult>, q_norm: &str) -> Vec<SearchResult> {
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
        hits
    }

    /// 对齐 commands::search_apps 的完整管线（缺内置设置页 UI，其余全量）：
    /// 空 Query 走固定+最近；非空走 内置项 → 应用召回 → 链接识别 → 文件 →
    /// 历史加权 → 网页搜索槽位；列表出全量（滚动加载在进程内直接滚动可见）。
    pub(super) fn refresh_results(&mut self) {
        let t0 = Instant::now();
        let q_norm = search::normalize_for_index(&self.query);
        if let Some(db) = &self.history {
            self.pinned = db.pinned_ids().into_iter().collect();
        }
        self.results = if q_norm.is_empty() {
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
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            search::order_by_recent(&index.apps, &recent, &pinned, search::MAX_RESULTS)
        } else {
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
            // 克隆同代索引 Arc 后立即释放锁，应用匹配不持全局锁。
            let retrieval = {
                let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
                index.retrieval.clone()
            };
            // 个性化在截断前进入统一排序：全量 Usage + 本 Query 配对 + Pin。
            let prefs = self.history.as_ref().map(|db| history::Personalization {
                usage: db.usage_all(),
                pairs: db.query_pairs_for(&q_norm),
                pinned: db.pinned_ids().into_iter().collect(),
                demoted: db.demoted_ids().into_iter().collect(),
                now: storage::now_ts(),
                query_norm: q_norm.clone(),
            });
            // 应用 + 系统入口：多路召回 → 验证 → 个性化 → 一次截断
            let mut hits = if let Some(ret) = retrieval {
                search::search_with_personalization(
                    &ret,
                    &self.query,
                    &user_targets,
                    prefs.as_ref(),
                    search::MAX_RESULTS,
                )
            } else {
                let (apps, system_entries) = {
                    let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
                    (index.apps.clone(), index.system_entries.clone())
                };
                search::search_system_personalized(
                    &apps,
                    &system_entries,
                    &self.query,
                    &user_targets,
                    prefs.as_ref(),
                    search::MAX_RESULTS,
                )
            };

            // 链接识别：网址 → 列已装浏览器直达（偏好优先）
            let preferred = self.history.as_ref().and_then(|h| h.preferred_browser());
            let search_template = self.history.as_ref().and_then(|h| h.search_url_template());
            let is_url = search::url::normalize_url(&self.query).is_some();
            if let Some(url) = search::url::normalize_url(&self.query) {
                let mut merged = app::web::build_hits(&url, preferred.as_deref(), &self.icon_dir);
                merged.append(&mut hits);
                hits = merged;
            }

            // Everything 在后台查询；这里只合并已完成的真实文件结果。
            if self.files_mode {
                hits.extend(
                    self.file_results
                        .iter()
                        .filter(|result| result.item.source != "everything-status")
                        .cloned(),
                );
            }

            // 应用主结果已在统一入口完成个性化与层排序；此处不再二次加分/纯分数重排。
            hits.truncate(search::MAX_RESULTS);

            // 网页搜索：非网址；有应用类结果时第 5 位固定「用浏览器搜索」，否则列浏览器
            if !is_url && !self.query.trim().is_empty() {
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

            // 成功嗅探到引擎模板则写回（副本库），避免每次读浏览器配置
            if search_template.is_none() {
                if let Some(pref) = preferred.as_deref() {
                    if let Some(t) = system::search_engine::detect_search_template(pref) {
                        if let Some(db) = self.history.as_mut() {
                            let _ = db.set_search_url_template(&t);
                        }
                    }
                }
            }

            hits
        };
        if self.files_mode {
            prepend_dependency_status(&mut self.results, &self.file_results);
        }
        self.selected = 0;
        // 列表刷新后回到顶部，避免选中行与滚动位置错位。
        self.hover_suppressed = false;
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
    use super::{
        build_file_results, everything_status_result, is_current_file_response,
        prepend_dependency_status,
    };
    use crate::system::everything::{self, Availability};

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
}
