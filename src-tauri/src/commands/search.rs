//! 搜索类 IPC：搜索、Alias 候选、索引计数。

use tauri::{AppHandle, State};

use crate::model::SearchResult;
use crate::search::UserTarget;
use crate::state::AppState;
use crate::{app, history, search, storage, system};

/// 文件搜索单次查询上限（滚动加载在缓存内翻页）。
const FILES_MAX: usize = 20;

#[tauri::command]
pub fn search_apps(
    query: String,
    include_files: Option<bool>,
    limit: Option<usize>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let files = include_files.unwrap_or(false);
    let limit = limit.unwrap_or(search::TOP_N).clamp(1, search::MAX_RESULTS);
    let q_norm = search::normalize_for_index(&query);

    // 滚动加载：同 Query 直接从完整排序缓存切片，不重搜、不重复查 Everything
    if let Some(page) = state.cache_get(&q_norm, files, limit) {
        return Ok(page);
    }

    // 空 Query：打开启动器时展示固定项 + 最近使用
    if q_norm.is_empty() {
        let (recent, pinned) = match state.history.lock() {
            Ok(h) => (
                h.recent_ids(search::MAX_RESULTS).unwrap_or_default(),
                h.pinned_ids(),
            ),
            Err(_) => (Vec::new(), Vec::new()),
        };
        // 索引只读：锁内直接搜，避免每个按键全量克隆索引
        let hits = {
            let index = state.index.lock().map_err(|e| e.to_string())?;
            search::order_by_recent(&index.apps, &recent, &pinned, search::MAX_RESULTS)
        };
        state.cache_put(q_norm, files, hits.clone());
        let end = limit.min(hits.len());
        return Ok(hits[..end].to_vec());
    }

    let user_targets: Vec<UserTarget> = state
        .history
        .lock()
        .map(|h| {
            h.alias_matches(&q_norm)
                .into_iter()
                .map(|a| UserTarget {
                    id: a.target_id,
                    name: a.target_name.to_lowercase(),
                })
                .collect()
        })
        .unwrap_or_default();

    let mut hits = {
        let index = state.index.lock().map_err(|e| e.to_string())?;
        search::search(&index.apps, &query, &user_targets, search::MAX_RESULTS)
    };

    let icon_dir = crate::state::icon_dir(&app);
    let preferred = state
        .history
        .lock()
        .ok()
        .and_then(|h| h.preferred_browser());
    let search_template = state
        .history
        .lock()
        .ok()
        .and_then(|h| h.search_url_template());
    let is_url = search::url::normalize_url(&query).is_some();

    // 内置：Kite 设置 + Windows 系统设置页
    if !q_norm.is_empty() {
        let mut builtins = app::builtin::collect_builtin_hits(&q_norm, &icon_dir);
        builtins.append(&mut hits);
        hits = builtins;
    }

    // 网址：列出已安装浏览器打开（偏好优先）
    if let Some(url) = search::url::normalize_url(&query) {
        let url_hits = app::web::build_hits(&url, preferred.as_deref(), &icon_dir);
        let mut merged = url_hits;
        merged.append(&mut hits);
        hits = merged;
    }

    // Everything 仅在用户打开「搜文件」时调用；走 SDK IPC，绝不拉起 Everything 主窗口。
    // 文件条数固定按上限取：完整排序结果进缓存后，滚动加载才可能翻到更多文件。
    if files && q_norm.len() >= 2 {
        for fh in system::everything::search_files(&query, FILES_MAX) {
            let name = fh.name.clone();
            let id = format!("file:{}", fh.path.to_lowercase());
            let mut item = crate::model::AppItem::scanned(id, name, fh.path, None, None, "everything");
            item.attach_search_fields();
            item.icon = system::icons::cache_type_icon(&icon_dir, &fh.name, fh.is_folder);
            hits.push(SearchResult {
                item,
                score: 400,
                matched_by: "file".into(),
            });
        }
    }

    // 个性化加权（历史 + 固定，Match 仍是主信号）：批量取历史后统一应用
    let ids: Vec<String> = hits.iter().map(|h| h.item.id.clone()).collect();
    let (usage_map, pair_map, pinned) = match state.history.lock() {
        Ok(hdb) => (
            hdb.usage_snapshot(&ids),
            hdb.query_pair_snapshot(&q_norm, &ids),
            hdb.pinned_ids().into_iter().collect(),
        ),
        Err(_) => Default::default(),
    };
    history::apply_boosts(&mut hits, usage_map, pair_map, &q_norm, storage::now_ts(), &pinned);

    hits = search::rerank(hits, search::MAX_RESULTS);

    // 非网址：有命中时第 5 位固定「用浏览器搜索」；无命中则列出各浏览器搜索
    if !is_url && !query.trim().is_empty() {
        let has_app_like = hits
            .iter()
            .any(|h| h.item.source != "browser" && h.item.source != "websearch");
        if has_app_like {
            if let Some(web) = app::web::build_primary_search_hit(
                query.trim(),
                preferred.as_deref(),
                &icon_dir,
                search_template.as_deref(),
            ) {
                hits = app::web::insert_at_slot(hits, web, app::web::WEB_SEARCH_SLOT);
            }
        } else {
            hits = app::web::build_search_hits(
                query.trim(),
                preferred.as_deref(),
                &icon_dir,
                search_template.as_deref(),
            );
            hits = search::rerank(hits, search::MAX_RESULTS);
        }
    }

    // 成功嗅探到引擎模板则缓存，避免每次读 Preferences
    if search_template.is_none() {
        if let Some(pref) = preferred.as_deref() {
            if let Some(t) = system::search_engine::detect_search_template(pref) {
                if let Ok(mut hdb) = state.history.lock() {
                    let _ = hdb.set_search_url_template(&t);
                }
            }
        }
    }

    state.cache_put(q_norm, files, hits.clone());
    let end = limit.min(hits.len());
    Ok(hits[..end].to_vec())
}

/// Alias 目标选择器：输入目标名称时返回索引候选 Top N（不传完整索引给前端）。
#[tauri::command]
pub fn search_alias_targets(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let limit = limit.unwrap_or(8).clamp(1, 20);
    let index = state.index.lock().map_err(|e| e.to_string())?;
    Ok(search::name_candidates(&index.apps, &query, limit))
}

#[tauri::command]
pub fn index_count(state: State<'_, AppState>) -> Result<usize, String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    Ok(index.apps.len())
}
