//! 搜索结果刷新与结果源合并。
//!
//! 职责拆分：
//! - 本文件：State 编排（请求、落地、合并、Provider / 空 Query 入口）
//! - `file_search`：Everything 文件结果与依赖状态
//! - `direct_path`：绝对路径直达动作
//! - `empty_query`：空 Query 默认列表组装

use super::*;
use crate::model::{ResultAction, ResultSource};
use std::sync::Arc;

mod direct_path;
mod empty_query;
mod file_search;
#[cfg(test)]
mod tests;

pub(crate) use direct_path::{build_direct_path_results, prepend_direct_path_results};
pub(crate) use empty_query::build_empty_query_lists;
pub(crate) use file_search::{
    build_file_results, is_current_file_response, prepend_dependency_status,
};
#[cfg(test)]
pub(crate) use file_search::everything_status_result;

impl State {
    pub(super) fn request_direct_path(&mut self) {
        self.direct_path_generation = self.direct_path_generation.wrapping_add(1);
        self.direct_path_results.clear();
        self.direct_path_latest.store(
            self.direct_path_generation,
            std::sync::atomic::Ordering::Release,
        );
        let query = self.query.clone();
        if app::actions::direct_path_candidate(&query).is_none() {
            return;
        }
        let generation = self.direct_path_generation;
        let latest = self.direct_path_latest.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            if latest.load(std::sync::atomic::Ordering::Acquire) != generation {
                return;
            }
            let hits = build_direct_path_results(&query);
            send_event(Message::DirectPathReady(generation, query, hits));
        });
    }

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
            send_event(Message::FileSearchReady(
                generation, query, results, elapsed_us,
            ));
        });
    }

    /// 读取当前个性化快照（历史/Pin/降权），供后台 worker 使用。
    ///
    /// usage/pin/demote 走内存缓存，避免每次按键全表扫 `usage_history`；
    /// launch/pin/demote/清空历史后通过 `invalidate_prefs_cache` 重建。
    /// pairs 仍按当前 query 现查。
    pub(super) fn snapshot_personalization(
        &mut self,
        q_norm: &str,
    ) -> Option<history::Personalization> {
        let db = self.history.as_ref()?;
        if self.prefs_cache.is_none() {
            self.prefs_cache = Some(CachedPrefs {
                usage: db.usage_all(),
                pinned: db.pinned_ids().into_iter().collect(),
                demoted: db.demoted_ids().into_iter().collect(),
            });
        }
        let cached = self.prefs_cache.clone().unwrap_or_default();
        Some(history::Personalization {
            usage: cached.usage,
            pairs: db.query_pairs_for(q_norm),
            pinned: cached.pinned,
            demoted: cached.demoted,
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
                send_event(Message::AppSearchReady(
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
        // Trigger 只提供一个可选入口；插件查询在用户进入后才执行。
        let trigger = self.trigger_result();
        if let Some(hit) = trigger.as_ref() {
            let pos = hits
                .iter()
                .position(|h| h.score < hit.score)
                .unwrap_or(hits.len());
            hits.insert(pos, hit.clone());
        }
        // Command 静态入口：不启动插件进程；按分数插入，不无条件顶掉本机精确命中
        {
            let reg = self
                .plugin_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let commands = plugin::command_hits(&reg, self.query.trim());
            for cmd in commands.into_iter() {
                if let (
                    Some(trigger),
                    ResultAction::Plugin {
                        plugin_id, payload, ..
                    },
                ) = (&trigger, &cmd.action)
                {
                    if let ResultSource::Plugin {
                        plugin_id: trigger_id,
                        provider_id,
                    } = &trigger.source
                    {
                        if plugin_id == trigger_id
                            && payload.get("provider").and_then(|v| v.as_str()) == Some(provider_id)
                        {
                            continue;
                        }
                    }
                }
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
        prepend_direct_path_results(&mut hits, &self.direct_path_results);
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

    /// 最终列表组装的唯一实现。
    ///
    /// - 非文件模式：应用命中 → 网址直达 → 追加 Everything 文件 → 截断 → 网页搜索槽位
    /// - 文件模式：Everything 文件优先，再拼应用/网址等其余命中（依赖状态仍由调用方置顶）
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
            // 用户已显式进入文件模式：文件结果排在应用等其余命中之前。
            let mut files: Vec<SearchResult> = self
                .file_results
                .iter()
                .filter(|result| result.item.source != "everything-status")
                .cloned()
                .collect();
            if !files.is_empty() {
                files.append(&mut hits);
                hits = files;
            }
        }
        hits.truncate(search::MAX_RESULTS);
        if !is_url
            && !q_norm.is_empty()
            && app::actions::direct_path_candidate(&self.query).is_none()
        {
            // 文件模式已把 Everything 命中置顶：再插网页槽位会打散文件列表。
            let files_mode_has_files =
                self.files_mode && hits.iter().any(|h| h.item.source == "everything");
            if !files_mode_has_files {
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
    /// - 已进入的 Provider Mode：查询交给插件；尚未进入时 Trigger 作为普通搜索入口；
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
            if self.provider_mode.as_ref().is_some_and(|current| {
                current.plugin_id == act.plugin_id && current.provider_id == act.provider_id
            }) {
                self.app_query_generation = self.app_query_generation.wrapping_add(1);
                self.app_search_worker.cancel_current();
                self.results_stale = true;
                self.plugin_panel = None;
                self.plugin_flash = None;
                self.provider_mode = Some(act.clone());
                self.plugin_query_generation = self.plugin_query_generation.wrapping_add(1);
                let gen = self.plugin_query_generation;
                let provider_id = act.provider_id.clone();
                submit_plugin_query(self, gen, act);
                self.qlog(|| format!("provider mode enter {provider_id:?}"));
                return;
            }
        }

        // Trigger 不再命中：退出 Provider Mode，回到 Core Search
        if self.provider_mode.is_some() {
            self.exit_provider_mode();
        }

        let q_norm = search::normalize_for_index(&self.query);
        if q_norm.is_empty() && !self.query.trim().is_empty() {
            self.results = self.trigger_result().into_iter().collect();
            self.results_stale = false;
            self.selected = 0;
            self.navigation_mode = NavigationMode::Input;
            return;
        }
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
                    // 保持存储层顺序（pinned_at DESC），固定项截断才确定
                    h.pinned_ids(),
                )
            })
            .unwrap_or_default();

        let index_apps = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            index.apps.clone()
        };

        let lists = build_empty_query_lists(&recent_ids, &pinned_ids, &index_apps);
        self.grid_recent_count = lists.recent.len();
        let mut results = lists.recent;
        results.extend(lists.pinned);
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

    fn trigger_result(&self) -> Option<SearchResult> {
        let reg = self
            .plugin_registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let act = plugin::route_query(&self.query, &reg)?;
        let plugin = reg.get(&act.plugin_id)?;
        let title = plugin.manifest.contributes.commands.iter()
            .find(|cmd| matches!(&cmd.action, plugin::manifest::CommandAction::EnterProvider { provider } if provider == &act.provider_id))
            .map(|cmd| cmd.title.clone())
            .unwrap_or_else(|| format!("{} · {}", plugin.manifest.plugin.name, act.provider_id));
        let mut item = AppItem::scanned(
            format!("plugin-trigger:{}:{}", act.plugin_id, act.provider_id),
            title,
            "Enter 进入插件".into(),
            None,
            None,
            "plugin-command",
        );
        item.attach_search_fields();
        Some(
            SearchResult::scored(item, 900, "plugin-command").with_source_action(
                ResultSource::Plugin {
                    plugin_id: act.plugin_id.clone(),
                    provider_id: act.provider_id.clone(),
                },
                ResultAction::plugin(
                    &act.plugin_id,
                    "enter_trigger_provider",
                    serde_json::json!({"provider": act.provider_id}),
                ),
            ),
        )
    }

    pub(super) fn exit_provider_mode(&mut self) {
        self.provider_mode = None;
        self.plugin_panel = None;
        self.plugin_query_generation = self.plugin_query_generation.wrapping_add(1);
        self.plugin_query_worker.cancel_current();
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

/// Provider Mode：把查询交给常驻 worker，不在 UI 线程临时 spawn。
fn submit_plugin_query(state: &State, generation: u64, act: plugin::Activation) {
    let job = plugin::PluginQueryJob {
        generation,
        activation: act,
        registry: state.plugin_registry.clone(),
        host: state.plugin_host.clone(),
        on_host_call: Arc::new(|pid, call| send_event(Message::PluginHostCall(pid, call))),
        on_done: Arc::new(|generation, payload| {
            send_event(Message::PluginQueryReady(generation, payload))
        }),
    };
    state.plugin_query_worker.submit(job);
}
