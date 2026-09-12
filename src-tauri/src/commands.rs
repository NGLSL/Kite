//! React 与 Rust 的 IPC 面。业务逻辑放在同级模块，本文件只做转发。

use tauri::{AppHandle, Emitter, Manager, State};

use crate::model::{AppItem, SearchResult};
use crate::state::{rebuild_index, AppState};
use crate::storage::settings::{Settings, UserAlias};
use crate::{app, history, search, storage, system};

#[tauri::command]
pub fn search_apps(
    query: String,
    include_files: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let q_norm = search::normalize_for_index(&query);
    // 克隆后尽快释放锁，避免搜索/历史 IO 堵住 UI
    let apps: Vec<_> = {
        let index = state.index.lock().map_err(|e| e.to_string())?;
        index.apps.clone()
    };

    // 空 Query：打开启动器时展示最近使用
    if q_norm.is_empty() {
        let recent = state
            .history
            .lock()
            .ok()
            .and_then(|h| h.recent_ids(search::TOP_N).ok())
            .unwrap_or_default();
        return Ok(search::order_by_recent(&apps, &recent, search::TOP_N));
    }

    let user_targets: Vec<String> = state
        .history
        .lock()
        .map(|h| {
            h.alias_targets(&q_norm)
                .into_iter()
                .map(|t| t.to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    let mut hits = search::search(&apps, &query, &user_targets);

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

    // Everything 仅在用户打开「搜文件」时调用，绝不默认拉起
    if include_files.unwrap_or(false) && q_norm.len() >= 2 {
        let max_files = 5usize;
        for fh in system::everything::search_files(&query, max_files) {
            let name = fh.name.clone();
            let id = format!("file:{}", fh.path.to_lowercase());
            let mut item = AppItem::scanned(id, name, fh.path, None, None, "everything");
            item.attach_search_fields();
            hits.push(SearchResult {
                item,
                score: 400,
                matched_by: "file".into(),
            });
        }
    }

    // 历史加权（Match 仍是主信号）
    if !q_norm.is_empty() {
        if let Ok(hdb) = state.history.lock() {
            let now = storage::now_ts();
            for hit in &mut hits {
                let usage = hdb.usage(&hit.item.id).unwrap_or_default();
                let pair = hdb.query_pair(&q_norm, &hit.item.id).unwrap_or_default();
                let boost = history::history_boost(&q_norm, &usage, &pair, now);
                hit.score += boost;
                if boost > 0 {
                    hit.matched_by = format!("{}+history", hit.matched_by);
                }
            }
        }
    }

    hits = search::rerank(hits, search::TOP_N);

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
                hits.truncate(search::TOP_N);
            }
        } else {
            hits = app::web::build_search_hits(
                query.trim(),
                preferred.as_deref(),
                &icon_dir,
                search_template.as_deref(),
            );
            hits = search::rerank(hits, search::TOP_N);
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

    Ok(hits)
}

#[tauri::command]
pub fn index_count(state: State<'_, AppState>) -> Result<usize, String> {
    let index = state.index.lock().map_err(|e| e.to_string())?;
    Ok(index.apps.len())
}

#[tauri::command]
pub fn launch_app(
    id: String,
    query: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 内置动作：打开 Kite 设置（不关窗）
    if id == "kite:settings" {
        state.set_settings_open(true);
        let _ = app.emit_to("main", "kite://open-settings", ());
        system::window::set_settings_mode(&app, true);
        return Ok(());
    }

    // 浏览器打开网址 / 网页搜索
    if let Some((kind, browser_id, payload)) = app::web::parse_id(&id) {
        let cached = state
            .history
            .lock()
            .ok()
            .and_then(|h| h.search_url_template());
        let preferred = if kind == "websearch" {
            app::web::launch_websearch(&browser_id, &payload, cached.as_deref())?
        } else {
            app::web::launch_url(&browser_id, &payload)?
        };
        let q_norm = search::normalize_for_index(&query);
        if let Ok(mut hdb) = state.history.lock() {
            let now = storage::now_ts();
            let _ = hdb.record_launch(&id, &q_norm, now);
            if let Some(pid) = preferred {
                let _ = hdb.set_preferred_browser(&pid);
            }
        }
        system::window::hide(&app);
        return Ok(());
    }

    // Everything 文件结果：直接按路径打开
    if let Some(path) = id.strip_prefix("file:") {
        let path = path.replace('/', "\\");
        app::launch(&AppItem::scanned(
            id.clone(),
            path.clone(),
            path,
            None,
            None,
            "everything",
        ))?;
        let q_norm = search::normalize_for_index(&query);
        if let Ok(mut hdb) = state.history.lock() {
            let now = storage::now_ts();
            let _ = hdb.record_launch(&id, &q_norm, now);
        }
        system::window::hide(&app);
        return Ok(());
    }

    let index = state.index.lock().map_err(|e| e.to_string())?;
    let item = index
        .apps
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "app not found".to_string())?;
    let id = item.id.clone();
    app::launch(item)?;

    let q_norm = search::normalize_for_index(&query);
    if let Ok(mut hdb) = state.history.lock() {
        let now = storage::now_ts();
        if let Err(e) = hdb.record_launch(&id, &q_norm, now) {
            eprintln!("history write failed: {e}");
        }
    }

    system::window::hide(&app);
    Ok(())
}

#[tauri::command]
pub fn set_ui_mode(
    settings: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.set_settings_open(settings);
    system::window::set_settings_mode(&app, settings);
    Ok(())
}

#[tauri::command]
pub fn rescan_apps(app: AppHandle) -> Result<usize, String> {
    rebuild_index(&app)
}

#[tauri::command]
pub fn toggle_window(app: AppHandle) {
    system::window::toggle(&app);
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let hdb = state.history.lock().map_err(|e| e.to_string())?;
    Ok(hdb.load_settings())
}

#[tauri::command]
pub fn save_settings(
    hide_on_blur: Option<bool>,
    autostart: Option<bool>,
    max_results: Option<i64>,
    hotkey: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Settings, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    let mut s = hdb.load_settings();
    let mut warn: Option<String> = None;

    if let Some(v) = hide_on_blur {
        s.hide_on_blur = v;
        let _ = hdb.save_setting("hide_on_blur", if v { "1" } else { "0" });
        crate::log::info(&format!("save hide_on_blur={v}"));
    }
    if let Some(v) = autostart {
        s.autostart = v;
        let _ = hdb.save_setting("autostart", if v { "1" } else { "0" });
        if let Err(e) = system::autostart::set_autostart(v) {
            // 写库成功但注册表失败：提示但不整体回滚
            warn = Some(format!("开机启动写入失败: {e}"));
            crate::log::info(&warn.clone().unwrap_or_default());
        } else {
            crate::log::info(&format!("save autostart={v}"));
        }
    }
    if let Some(v) = max_results {
        let v = v.clamp(5, 30);
        s.max_results = v;
        let _ = hdb.save_setting("max_results", &v.to_string());
        crate::log::info(&format!("save max_results={v}"));
    }
    if let Some(h) = hotkey {
        let h = h.trim().to_string();
        if system::hotkey::parse_hotkey(&h).is_none() {
            return Err(format!("无法识别的快捷键: {h}"));
        }
        drop(hdb);
        let label = system::hotkey::reregister(&app, &h)?;
        let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
        let _ = hdb.save_setting("hotkey", &h);
        s.hotkey = h;
        s.hotkey_label = label;
        crate::log::info("save hotkey ok");
    }
    let _ = app.emit("kite://settings-changed", ());
    if let Some(w) = warn {
        return Err(w);
    }
    Ok(s)
}

#[tauri::command]
pub fn list_user_aliases(state: State<'_, AppState>) -> Result<Vec<UserAlias>, String> {
    state
        .history
        .lock()
        .map_err(|e| e.to_string())?
        .list_aliases()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_user_alias(
    alias: String,
    target_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<UserAlias>, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.set_alias(&alias, &target_name).map_err(|e| e.to_string())?;
    hdb.list_aliases().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_user_alias(
    alias: String,
    state: State<'_, AppState>,
) -> Result<Vec<UserAlias>, String> {
    let mut hdb = state.history.lock().map_err(|e| e.to_string())?;
    hdb.remove_alias(&alias).map_err(|e| e.to_string())?;
    hdb.list_aliases().map_err(|e| e.to_string())
}
