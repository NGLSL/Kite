//! 搜索结果刷新与已有结果源合并。

use super::*;

impl State {
    /// 使当前文件结果失效，并在需要时把 Everything 查询放到后台线程。
    pub(super) fn request_file_search(&mut self) {
        self.file_query_generation = self.file_query_generation.wrapping_add(1);
        self.file_results.clear();
        let query = self.query.trim().to_string();
        if !self.files_mode || search::normalize_for_index(&query).chars().count() < 2 {
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
            // 应用 + 系统入口统一多路召回（快照同代索引，不按来源截断前排）
            // 系统入口图标已在快照准备阶段写入，按键路径不再提取/校验
            let mut hits = if let Some(ret) = retrieval {
                search::search_with_index(&ret, &self.query, &user_targets, search::MAX_RESULTS)
            } else {
                let (apps, system_entries) = {
                    let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
                    (index.apps.clone(), index.system_entries.clone())
                };
                search::search_with_system(
                    &apps,
                    &system_entries,
                    &self.query,
                    &user_targets,
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

            // Everything 在后台查询；这里只合并当前 query 已完成的不可变结果。
            if self.files_mode && q_norm.chars().count() >= 2 {
                hits.extend(self.file_results.iter().cloned());
            }

            // 个性化加权（历史 + 固定，Match 仍是主信号）
            if let Some(db) = &self.history {
                let ids: Vec<String> = hits.iter().map(|h| h.item.id.clone()).collect();
                let usage = db.usage_snapshot(&ids);
                let pairs = db.query_pair_snapshot(&q_norm, &ids);
                let pinned = db.pinned_ids().into_iter().collect();
                history::apply_boosts(&mut hits, usage, pairs, &q_norm, storage::now_ts(), &pinned);
            }
            hits = search::rerank(hits, search::MAX_RESULTS);

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
        self.selected = 0;
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
    system::everything::search_files(query, 20)
        .into_iter()
        .map(|hit| {
            let id = format!("file:{}", hit.path.to_lowercase());
            let mut item =
                AppItem::scanned(id, hit.name.clone(), hit.path, None, None, "everything");
            item.attach_search_fields();
            item.icon = system::icons::cache_type_icon(icon_dir, &hit.name, hit.is_folder);
            SearchResult {
                item,
                score: 400,
                matched_by: "file".into(),
            }
        })
        .collect()
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
    use super::is_current_file_response;

    #[test]
    fn stale_or_disabled_file_results_are_rejected() {
        assert!(is_current_file_response(true, 4, "report", 4, "report"));
        assert!(!is_current_file_response(true, 5, "report", 4, "report"));
        assert!(!is_current_file_response(true, 4, "reports", 4, "report"));
        assert!(!is_current_file_response(false, 4, "report", 4, "report"));
    }
}
