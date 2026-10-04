//! 搜索窗口的选择、启动、设置和快捷键动作。

use super::*;

/// Build the single action list shared by mouse and Shift+F10.
pub(super) fn menu_entries(state: &State, item: &AppItem) -> Vec<(String, MenuAction)> {
    let target_is_fs = std::path::Path::new(&item.target).is_file()
        || std::path::Path::new(&item.target).is_dir();
    let mut entries = Vec::new();
    if target_is_fs {
        entries.push(("打开所在文件夹".to_string(), MenuAction::OpenFolder));
        entries.push(("复制路径".to_string(), MenuAction::CopyPath));
    }
    entries.push(("复制名称".to_string(), MenuAction::CopyName));
    if can_personalize_entry(item) {
        let pinned = state.pinned.contains(&item.id);
        entries.push((
            if pinned { "取消固定" } else { "固定" }.to_string(),
            MenuAction::TogglePin,
        ));
        let demoted = state
            .history
            .as_ref()
            .map(|db| db.is_demoted(&item.id))
            .unwrap_or(false);
        entries.push((
            if demoted { "恢复优先级" } else { "降低此结果优先级" }.to_string(),
            if demoted { MenuAction::Undemote } else { MenuAction::Demote },
        ));
    }
    if is_app_entry(item) {
        entries.push(("设置 Alias".to_string(), MenuAction::SetAlias));
        entries.push(("隐藏此入口".to_string(), MenuAction::HideEntry));
    }
    // Once the panel is open, use the query captured with its target.  The
    // input may change while a mouse or keyboard action is pending, but the
    // panel must continue to describe the action that will be executed.
    let panel_query = if state.menu.is_some() {
        state.menu_query.trim()
    } else {
        state.query.trim()
    };
    if !panel_query.is_empty() && !state.files_mode && state.provider_mode.is_none() {
        entries.push((
            format!("忘记「{panel_query}」的学习记录"),
            MenuAction::ForgetQuery,
        ));
    }
    if item.source == "direct-path" && is_registerable_path(&item.target) {
        entries.push(("添加到 Kite".to_string(), MenuAction::AddToKite));
    }
    entries
}

pub(super) fn is_app_entry(item: &AppItem) -> bool {
    !item.id.starts_with("kite:")
        && crate::model::is_hideable_application_source(&item.source)
}

/// Pin/Demote keep the original panel permissions. Alias/Hide use the
/// narrower application-source check above, while file, web and plugin
/// results may still carry their existing preference actions.
fn can_personalize_entry(item: &AppItem) -> bool {
    item.source != "everything-status" && item.source != "direct-path"
}

fn resolve_from_index(state: &State, captured: &AppItem) -> Option<AppItem> {
    let index = state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    index
        .apps
        .iter()
        .chain(index.system_entries.iter())
        .find(|candidate| candidate.id == captured.id)
        .cloned()
        .filter(|current| same_launch_identity(current, captured))
}

fn needs_index_resolution(item: &AppItem, action: MenuAction) -> bool {
    if matches!(action, MenuAction::SetAlias | MenuAction::HideEntry) {
        return true;
    }
    matches!(action, MenuAction::TogglePin | MenuAction::Demote | MenuAction::Undemote)
        && !item.id.starts_with("kite:")
        && (is_app_entry(item)
            || matches!(
                item.source.as_str(),
                "builtin" | "builtin-system" | "win-settings"
            ))
}

pub(super) fn is_registerable_path(target: &str) -> bool {
    let path = std::path::Path::new(target);
    path.is_file()
        && matches!(
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_ascii_lowercase())
                .as_deref(),
            Some("exe" | "lnk")
        )
}

pub(super) fn close_menu(state: &mut State) {
    state.menu = None;
    state.menu_selected = 0;
    state.menu_query.clear();
    state.action_alias_item = None;
    state.action_alias_input.clear();
    state.action_alias_conflict = None;
}

pub(super) fn cancel_action_alias(state: &mut State) -> Task<Message> {
    state.action_alias_item = None;
    state.action_alias_input.clear();
    state.action_alias_conflict = None;
    iced::widget::operation::focus(search_view::input_id())
}

pub(super) fn menu_height(state: &State, item: &AppItem) -> f32 {
    if state.action_alias_item.is_some() {
        if state.action_alias_conflict.is_some() {
            220.0
        } else {
            145.0
        }
    } else {
        8.0 + menu_entries(state, item).len() as f32 * 33.0
    }
}

/// Keep an already-open panel inside the window after its content expands.
/// Alias conflict details add a second block below the input, so the anchor
/// captured for the compact menu may need to move upward before rendering.
pub(super) fn clamp_menu_height(state: &mut State, height: f32) {
    if let Some((_, _, y)) = state.menu.as_mut() {
        *y = (*y).min(WINDOW_H - height).max(8.0);
    }
}

pub(super) fn open_context_menu(state: &mut State, index: usize) -> Task<Message> {
    if state.menu.is_some() {
        close_menu(state);
        return Task::none();
    }
    let Some(item) = super::interaction::capture_menu_item(&state.results, index) else {
        return Task::none();
    };
    state.menu_query = state.query.clone();
    let panel_height = menu_height(state, &item);
    let c = state.cursor.get();
    let x = c.x.min(WINDOW_W - 240.0).max(8.0);
    let y = c.y.min(WINDOW_H - panel_height).max(8.0);
    state.menu = Some((item, x, y));
    state.menu_selected = 0;
    Task::none()
}

fn same_launch_identity(left: &AppItem, right: &AppItem) -> bool {
    crate::app::scanner::util::launch_identity(&left.target, left.args.as_deref())
        == crate::app::scanner::util::launch_identity(&right.target, right.args.as_deref())
}

pub(super) fn resolve_current_item(state: &State, captured: &AppItem) -> Option<AppItem> {
    resolve_from_index(state, captured)
}

/// 上下文菜单动作。
pub(super) fn menu_action(state: &mut State, item: AppItem, action: MenuAction) -> Task<Message> {
    // Keep the Query captured at panel-open time before close_menu clears it.
    // Query-level forgetting must never follow text the user typed afterwards.
    let captured_query = state.menu_query.clone();
    let item = if needs_index_resolution(&item, action) {
        let Some(current) = resolve_current_item(state, &item) else {
            close_menu(state);
            return flash(state, "目标已不在当前索引，未执行操作");
        };
        current
    } else {
        item
    };
    if !matches!(action, MenuAction::SetAlias) {
        close_menu(state);
    }
    match action {
        MenuAction::OpenFolder => {
            let r = app::actions::open_containing_folder(&item.target);
            state.qlog(|| format!("ctx open_folder err={r:?}"));
            if let Err(error) = r {
                return flash(state, &format!("打开所在文件夹失败：{error}"));
            }
        }
        MenuAction::CopyPath => return iced::clipboard::write(item.target),
        MenuAction::CopyName => return iced::clipboard::write(item.display_name),
        MenuAction::TogglePin => {
            let Some(db) = &mut state.history else {
                return flash(state, "历史库不可用，无法修改固定状态");
            };
            let r = if state.pinned.contains(&item.id) {
                db.unpin_item(&item.id)
            } else {
                db.pin_item(&item.id, storage::now_ts())
            };
            state.qlog(|| format!("ctx pin toggle ok={}", r.is_ok()));
            if let Err(error) = r {
                return flash(state, &format!("保存固定状态失败：{error}"));
            }
            state.invalidate_prefs_cache();
            state.refresh_results();
        }
        MenuAction::Demote | MenuAction::Undemote => {
            let Some(db) = &mut state.history else {
                return flash(state, "历史库不可用，无法修改优先级");
            };
            let r = if matches!(action, MenuAction::Demote) {
                db.demote_item(&item.id, storage::now_ts())
            } else {
                db.undemote_item(&item.id)
            };
            state.qlog(|| {
                format!(
                    "ctx demote action={action:?} ok={} id={}",
                    r.is_ok(),
                    item.id
                )
            });
            if let Err(error) = r {
                return flash(state, &format!("保存优先级失败：{error}"));
            }
            state.invalidate_prefs_cache();
            state.refresh_results();
        }
        MenuAction::SetAlias => {
            if !is_app_entry(&item) {
                return flash(state, "当前结果不支持 Alias");
            }
            state.action_alias_item = Some(item);
            state.action_alias_input = state.menu_query.trim().to_string();
            state.action_alias_conflict = None;
            state.menu_selected = 0;
            clamp_menu_height(state, 145.0);
            return iced::widget::operation::focus(search_view::action_alias_input_id());
        }
        MenuAction::HideEntry => {
            if !is_app_entry(&item) {
                return flash(state, "当前结果不支持隐藏");
            }
            let Some(db) = state.history.as_mut() else {
                return flash(state, "历史库不可用，无法隐藏入口");
            };
            match db.hide_item(&item, storage::now_ts()) {
                Ok(()) => {
                    load_hidden(state);
                    let hidden_ids = state.hidden_ids.clone();
                    state.results.retain(|result| {
                        !hidden_ids.contains(&result.item.id)
                            || !crate::model::is_hideable_application_source(
                                &result.item.source,
                            )
                    });
                    if state.results.is_empty() {
                        state.selected = 0;
                    } else {
                        state.selected = state.selected.min(state.results.len() - 1);
                    }
                    state.invalidate_prefs_cache();
                    state.refresh_results();
                    return flash(state, "已隐藏此入口");
                }
                Err(error) => return flash(state, &format!("隐藏失败：{error}")),
            }
        }
        MenuAction::ForgetQuery => {
            let query_norm = search::normalize_for_index(captured_query.trim());
            if query_norm.is_empty() {
                return flash(state, "空查询没有可清除的学习记录");
            }
            let Some(db) = state.history.as_mut() else {
                return flash(state, "历史库不可用，无法清除查询学习");
            };
            match db.clear_query_pairs(&query_norm) {
                Ok(0) => return flash(state, "该查询没有可清除的学习记录"),
                Ok(n) => {
                    state.invalidate_prefs_cache();
                    state.refresh_results();
                    return flash(state, &format!("已清除该查询的 {n} 条学习记录"));
                }
                Err(error) => return flash(state, &format!("清除查询学习失败：{error}")),
            }
        }
        MenuAction::AddToKite => {
            if !is_registerable_path(&item.target) {
                return flash(state, "该路径不是可登记的 exe 或 lnk");
            }
            let path = item.target.clone();
            let name = std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&item.display_name)
                .to_string();
            let task = open_settings(state);
            state.settings_section = Section::Index;
            state.manual_path_input = path;
            state.manual_name_input = name;
            return task;
        }
    }
    Task::none()
}

pub(super) fn load_hidden(state: &mut State) {
    let Some(db) = &state.history else {
        state.hidden_items.clear();
        state.hidden_ids.clear();
        return;
    };
    match db.list_hidden_items() {
        Ok(items) => {
            state.hidden_ids = items.iter().map(|item| item.item_id.clone()).collect();
            state.hidden_items = items;
        }
        Err(error) => state.qlog(|| format!("load hidden entries failed: {error}")),
    }
}

pub(super) fn load_manual_apps(state: &mut State) {
    let Some(db) = &state.history else {
        state.manual_apps.clear();
        state.manual_status.clear();
        return;
    };
    match db.list_manual_apps() {
        Ok(items) => {
            state.manual_name_edits = items
                .iter()
                .map(|item| (item.id, item.display_name.clone()))
                .collect();
            state.manual_status = items
                .iter()
                .map(|item| {
                    let status = crate::app::manual::validate_entry(&item.path, &item.display_name)
                        .map(|_| "可用".to_string())
                        .unwrap_or_else(|error| format!("暂不可用：{error}"));
                    (item.id, status)
                })
                .collect();
            state.manual_apps = items;
        }
        Err(error) => state.qlog(|| format!("load manual apps failed: {error}")),
    }
}

pub(super) fn sync_manual_apps_to_scan_options(state: &mut State) {
    let mut options = state
        .scan_options
        .write()
        .unwrap_or_else(|error| error.into_inner());
    options.manual_apps = state.manual_apps.clone();
}

/// 键盘：输入模式下 ↑↓ 进入结果导航，结果导航模式下 ↑↓←→ 选择，
/// Enter 启动（组合态禁止，见 ime_composing），Esc 返回输入框或隐藏窗口；
/// Alt+1..9 启动对应项；热键录制态优先捕获。
pub(super) fn on_key(
    state: &mut State,
    key: Key,
    physical: Physical,
    mods: Modifiers,
) -> Task<Message> {
    if state.hidden {
        return Task::none();
    }
    if state.hotkey_recording {
        return hotkey_record_key(state, key, mods);
    }
    // Settings and standalone tool windows own their text/button keyboard
    // interaction. Keep only Escape global so captured widget events cannot
    // move or launch a result in the hidden/secondary UI.
    if (state.settings_open || state.any_tool_open())
        && !matches!(key, Key::Named(Named::Escape))
    {
        return Task::none();
    }

    if state.menu.is_some() {
        match key {
            Key::Named(Named::Escape) if !state.ime_composing => {
                if state.action_alias_item.is_some() {
                    return cancel_action_alias(state);
                } else {
                    close_menu(state);
                    return focus(search_view::input_id());
                }
            }
            Key::Named(Named::ArrowUp) => {
                let count = state
                    .menu
                    .as_ref()
                    .map(|(item, _, _)| menu_entries(state, item).len())
                    .unwrap_or(0);
                if count > 0 {
                    state.menu_selected = if state.menu_selected == 0 {
                        count - 1
                    } else {
                        state.menu_selected - 1
                    };
                }
                return Task::none();
            }
            Key::Named(Named::ArrowDown) => {
                let count = state
                    .menu
                    .as_ref()
                    .map(|(item, _, _)| menu_entries(state, item).len())
                    .unwrap_or(0);
                if count > 0 {
                    state.menu_selected = (state.menu_selected + 1) % count;
                }
                return Task::none();
            }
            Key::Named(Named::Enter) if !state.ime_composing => {
                if state.action_alias_item.is_some() {
                    return super::update_search::handle(state, Message::ActionAliasSave);
                }
                let selected = state.menu_selected;
                let chosen = state.menu.as_ref().and_then(|(item, _, _)| {
                    menu_entries(state, item).get(selected).map(|(_, action)| *action)
                });
                if let (Some((item, _, _)), Some(action)) = (state.menu.clone(), chosen) {
                    return menu_action(state, item, action);
                }
                return Task::none();
            }
            _ => return Task::none(),
        }
    }

    if matches!(key, Key::Named(Named::F10))
        && mods.shift()
        && !mods.control()
        && !mods.alt()
        && !state.settings_open
        && state.provider_mode.is_none()
        && !state.ime_composing
    {
        return open_context_menu(state, state.selected);
    }

    // Alt+1..9：兼容 modifiers.alt() / 本地 alt_down / 逻辑字符 / 物理 Digit|Numpad
    let alt_idx = alt_digit_index(&key, physical, mods, state.alt_down);
    if let Some(i) = alt_idx {
        let alt_down = state.alt_down;
        let mods_alt = mods.alt();
        state.qlog(|| {
            format!("alt-n idx={i} alt_down={alt_down} mods_alt={mods_alt} key={key:?} phys={physical:?}")
        });
        return launch_alt_digit(state, i);
    }

    match key {
        Key::Named(Named::Escape) if !state.ime_composing => {
            // 前端行为：菜单开着时 Esc 只关菜单；设置页开着时 Esc 回搜索
            if state.menu.is_some() {
                close_menu(state);
                state.qlog(|| "ctx menu closed (esc)".to_owned());
                return focus(search_view::input_id());
            }
            // JSON 工具窗：Esc 关闭工具窗，主窗不动。
            if state.any_tool_open() {
                if state.tools.base64.window_id.is_some() {
                    return super::tools::close_tool_panel(state, super::tools::ToolKind::Base64);
                } else if state.tools.hash.window_id.is_some() {
                    return super::tools::close_tool_panel(state, super::tools::ToolKind::Hash);
                } else {
                    return super::tools::close_tool_panel(state, super::tools::ToolKind::Json);
                }
            }
            // 二次确认：Esc 取消，不开工具窗。
            if state.pending_tool_confirm.is_some() {
                return super::tools::cancel_confirm(state);
            }
            if state.settings_open {
                return close_settings(state);
            }
            if state.navigation_mode == NavigationMode::Results {
                state.navigation_mode = NavigationMode::Input;
                state.hover_suppressed = false;
                return focus(search_view::input_id());
            }
            // Provider Mode：Esc 先退出到 Core Search，不直接隐藏启动器。
            if state.provider_mode.is_some() {
                state.qlog(|| "provider mode exit (esc)".to_owned());
                state.query.clear();
                state.request_direct_path();
                state.exit_provider_mode();
                state.refresh_results();
                return focus(search_view::input_id());
            }
            state.qlog(|| "hide issued (esc)".to_owned());
            hide(state);
            hide_task(state)
        }
        Key::Named(Named::ArrowUp) => {
            if state.navigation_mode == NavigationMode::Input {
                begin_result_navigation(state, true)
            } else if state.query.trim().is_empty() {
                move_selection(state, -8)
            } else {
                move_selection_query(state, &Named::ArrowUp)
            }
        }
        Key::Named(Named::ArrowDown) => {
            if state.navigation_mode == NavigationMode::Input {
                begin_result_navigation(state, false)
            } else if state.query.trim().is_empty() {
                move_selection(state, 8)
            } else {
                move_selection_query(state, &Named::ArrowDown)
            }
        }
        Key::Named(Named::ArrowLeft) => {
            if state.navigation_mode != NavigationMode::Results {
                Task::none()
            } else if state.query.trim().is_empty() {
                move_selection(state, -1)
            } else {
                move_selection_query(state, &Named::ArrowLeft)
            }
        }
        Key::Named(Named::ArrowRight) => {
            if state.navigation_mode != NavigationMode::Results {
                Task::none()
            } else if state.query.trim().is_empty() {
                move_selection(state, 1)
            } else {
                move_selection_query(state, &Named::ArrowRight)
            }
        }
        Key::Named(Named::Backspace) if !state.ime_composing => {
            if state.query.is_empty() {
                if state.provider_mode.is_some() {
                    state.exit_provider_mode();
                    state.refresh_results();
                    return focus(search_view::input_id());
                } else if state.files_mode {
                    state.files_mode = false;
                    state.file_filter = system::everything::FileFilter::All;
                    state.request_file_search();
                    state.refresh_results();
                    return Task::batch([sync_scroll(state), focus(search_view::input_id())]);
                }
            }
            Task::none()
        }
        Key::Named(Named::Enter) if !state.ime_composing => {
            if mods.control() && !mods.alt() && !mods.shift() {
                if state.provider_mode.is_some() {
                    if let Some(panel) = state.plugin_panel.clone() {
                        if let Some(text) = panel.primary_value() {
                            return Task::batch([
                                iced::clipboard::write(text),
                                flash(state, "已复制并保留输入"),
                            ]);
                        }
                    }
                    return flash(state, "暂无可复制的计算结果");
                }
                if let Some(res) = state.results.get(state.selected) {
                    let target = &res.item.target;
                    let is_fs = std::path::Path::new(target).is_file()
                        || std::path::Path::new(target).is_dir();
                    if is_fs {
                        let _ = app::actions::open_containing_folder(target);
                        hide(state);
                        return hide_task(state);
                    } else {
                        return flash(state, "当前项目不是本地文件或目录");
                    }
                }
                return Task::none();
            }
            if web_search_hotkey_pressed(state, mods) {
                launch_browser_search(state)
            } else {
                launch_selected(state)
            }
        }
        Key::Named(Named::Alt) => Task::none(),
        Key::Named(name) => {
            state.qlog(|| format!("key named {name:?} ignored"));
            Task::none()
        }
        Key::Character(c) => {
            // 普通输入交给 text_input；这里只记日志（避免与 QueryChanged 双处理）
            let _ = c;
            Task::none()
        }
        _ => Task::none(),
    }
}

/// 窗口内网页搜索快捷键是否按下（默认 Ctrl+Enter；设置可改）。
fn web_search_hotkey_pressed(state: &State, mods: Modifiers) -> bool {
    let spec = system::hotkey::normalize_web_search_hotkey(&state.web_search_hotkey);
    match spec {
        "Ctrl+Enter" => mods.control() && !mods.alt() && !mods.shift() && !mods.logo(),
        "Alt+Enter" => mods.alt() && !mods.control() && !mods.shift() && !mods.logo(),
        "Shift+Enter" => mods.shift() && !mods.control() && !mods.alt() && !mods.logo(),
        "Ctrl+Shift+Enter" => mods.control() && mods.shift() && !mods.alt() && !mods.logo(),
        _ => false,
    }
}

/// 用当前 Query 直接打开浏览器搜索（不依赖列表选中项）。
fn launch_browser_search(state: &mut State) -> Task<Message> {
    let q = state.query.trim().to_string();
    if q.is_empty() || state.settings_open || state.hidden {
        return Task::none();
    }
    let preferred = state.history.as_ref().and_then(|h| h.preferred_browser());
    let template = state.history.as_ref().and_then(|h| h.search_url_template());
    let browser_id = preferred.as_deref().unwrap_or("default");
    let t0 = Instant::now();
    system::env::refresh_process_env();
    let result = app::web::launch_websearch(browser_id, &q, template.as_deref());
    match result {
        Ok(preferred_id) => {
            state.qlog(|| {
                format!(
                    "websearch hotkey via {browser_id} in {}us",
                    t0.elapsed().as_micros()
                )
            });
            if let Some(db) = &mut state.history {
                let qn = search::normalize_for_index(&q);
                let id = app::web::make_search_id(browser_id, &q);
                let _ = db.record_launch(&id, &qn, storage::now_ts());
                if let Some(pid) = preferred_id {
                    let _ = db.set_preferred_browser(&pid);
                }
            }
            state.invalidate_prefs_cache();
            launch_and_hide(state)
        }
        Err(e) => {
            state.qlog(|| format!("websearch hotkey failed: {e}"));
            Task::none()
        }
    }
}

pub(super) fn launch_alt_digit(state: &mut State, i: usize) -> Task<Message> {
    if state.ime_composing
        || state.settings_open
        || state.alt_digit_consumed
        || i >= state.results.len()
    {
        return Task::none();
    }
    state.alt_digit_consumed = true;
    // KeyPressed 可能晚于 text_input，保留按下 Alt 前的搜索文本。
    if let Some(before) = state.query_at_alt.clone() {
        if state.query != before {
            state.query = before;
            state.refresh_results();
            // 列表仍是按下 Alt 前那次搜索的结果——用户按 Alt+N 要的就是它。
            // 上面的重新提交只是为了文本与请求一致，不该让本次启动被判为过期。
            state.results_stale = false;
        }
    }
    state.selected = i;
    launch_selected(state)
}

/// 选中行滚入可视区（保留上方两行），对应前端滚动定位行为。
pub(super) fn sync_scroll(state: &State) -> Task<Message> {
    let row_idx = if state.query.trim().is_empty() {
        if state.selected < state.grid_recent_count {
            state.selected / 8
        } else {
            2
        }
    } else {
        (state.selected / 8 * 4) + (state.selected % 4)
    };
    let y = ((row_idx as f32) * search_view::ROW_STEP - 2.0 * search_view::ROW_STEP).max(0.0);
    scroll_to(search_view::scroll_id(), AbsoluteOffset { x: 0.0, y })
}

/// 从输入模式进入结果导航：向下从第一项开始，向上从最后一项开始。
fn begin_result_navigation(state: &mut State, from_bottom: bool) -> Task<Message> {
    if state.results.is_empty() {
        return Task::none();
    }
    state.navigation_mode = NavigationMode::Results;
    state.selected = if from_bottom {
        state.results.len() - 1
    } else {
        0
    };
    state.hover_suppressed = true;
    sync_scroll(state)
}

pub(super) fn move_selection(state: &mut State, delta: i32) -> Task<Message> {
    if state.results.is_empty() {
        return Task::none();
    }
    let len = state.results.len() as i32;
    let next = (state.selected as i32 + delta).clamp(0, len - 1);
    state.selected = next as usize;
    // scroll_to 会让鼠标底下换成另一行并触发 on_enter；先抑制悬停改选。
    state.hover_suppressed = true;
    sync_scroll(state)
}

/// 双列搜索结果二维键盘漫游（上/下在当前列移动，左/右跨列移动）
pub(super) fn move_selection_2col(state: &mut State, key: &Named) -> Task<Message> {
    if state.results.is_empty() {
        return Task::none();
    }
    let len = state.results.len();
    let cur = state.selected;
    let offset = cur % 8;
    let next = match key {
        Named::ArrowDown => {
            if offset == 3 || offset == 7 {
                cur + 5
            } else {
                cur + 1
            }
        }
        Named::ArrowUp => {
            if offset == 0 || offset == 4 {
                if cur >= 5 {
                    cur - 5
                } else {
                    cur
                }
            } else {
                cur.saturating_sub(1)
            }
        }
        Named::ArrowRight => {
            if offset < 4 {
                cur + 4
            } else {
                cur
            }
        }
        Named::ArrowLeft => {
            if offset >= 4 {
                cur.saturating_sub(4)
            } else {
                cur
            }
        }
        _ => cur,
    };
    let target = if next < len {
        next
    } else if matches!(key, Named::ArrowDown) && cur < len - 1 {
        len - 1
    } else {
        cur
    };
    state.selected = target;
    state.hover_suppressed = true;
    sync_scroll(state)
}

/// 非空查询结果的导航：少于一整列时视觉上是单列，四个方向都按列表顺序移动；
/// 出现第二列后再使用四行双列的二维移动规则。
fn move_selection_query(state: &mut State, key: &Named) -> Task<Message> {
    if state.results.len() <= 4 {
        return match key {
            Named::ArrowUp | Named::ArrowLeft => move_selection(state, -1),
            Named::ArrowDown | Named::ArrowRight => move_selection(state, 1),
            _ => Task::none(),
        };
    }
    move_selection_2col(state, key)
}

pub(super) fn launch_selected(state: &mut State) -> Task<Message> {
    // Enter / Alt+数字 / 鼠标点击共用入口。
    // Provider Mode 下 Panel 的 default action 优先（NativeAction 由 Kite 执行）。
    if state.provider_mode.is_some() {
        if let Some(panel) = state.plugin_panel.clone() {
            if let Some(native) = panel.default_native_action() {
                return exec_native_panel_action(state, native);
            }
            if let Some(act) = panel.default_plugin_action() {
                return exec_result_action(state, act);
            }
            if let Some(val) = panel.primary_value() {
                return exec_native_panel_action(
                    state,
                    crate::plugin::panel::NativeAction::CopyText(val),
                );
            }
        }
    }
    if !state.results_are_launchable() {
        state.qlog(|| "launch ignored: results belong to a previous query".to_owned());
        return Task::none();
    }
    let Some(result) = state.results.get(state.selected).cloned() else {
        state.qlog(|| "enter with empty results; ignored".to_owned());
        return Task::none();
    };
    close_menu(state);
    exec_result_action(state, result.action.clone())
}

/// 统一执行 ResultAction：Result 管展示，Action 管行为。
pub(super) fn exec_result_action(
    state: &mut State,
    action: crate::model::ResultAction,
) -> Task<Message> {
    use crate::model::ResultAction;
    match action {
        ResultAction::CopyText { text } => {
            state.qlog(|| format!("native copy_text len={}", text.len()));
            iced::clipboard::write(text)
        }
        ResultAction::OpenFile { path } => {
            if !plugin::plugin_path_allowed(&path) {
                state.qlog(|| format!("open_file rejected unsafe path len={}", path.len()));
                return flash(state, "路径无效");
            }
            let t0 = Instant::now();
            system::env::refresh_process_env();
            match app::uwp::launch_shell_path(&path) {
                Ok(()) => {
                    state.qlog(|| format!("open_file {path} in {}us", t0.elapsed().as_micros()));
                    launch_and_hide(state)
                }
                Err(e) => {
                    state.qlog(|| format!("open_file failed: {e}"));
                    flash(state, "无法打开该路径")
                }
            }
        }
        ResultAction::OpenLocalPath { path } => exec_direct_path(state, &path, false),
        ResultAction::RevealPath { path } => exec_direct_path(state, &path, true),
        ResultAction::OpenUrl { url } => {
            if !plugin::plugin_url_allowed(&url) {
                state.qlog(|| "open_url rejected non-http(s)".to_owned());
                return flash(state, "链接无效");
            }
            let t0 = Instant::now();
            system::env::refresh_process_env();
            match app::uwp::launch_shell_path(&url) {
                Ok(()) => {
                    state.qlog(|| format!("open_url in {}us", t0.elapsed().as_micros()));
                    launch_and_hide(state)
                }
                Err(e) => {
                    state.qlog(|| format!("open_url failed: {e}"));
                    flash(state, "无法打开链接")
                }
            }
        }
        ResultAction::Plugin {
            plugin_id,
            action_id,
            payload,
        } => exec_plugin_action(state, &plugin_id, &action_id, payload),
        ResultAction::LaunchApp { item_id } => {
            // 只允许当前结果中已验证的 item_id，禁止回退到选中行造成误启动。
            let Some(item) = state
                .results
                .iter()
                .find(|r| r.item.id == item_id)
                .map(|r| r.item.clone())
            else {
                state.qlog(|| format!("launch skipped: item_id not in results {item_id}"));
                return Task::none();
            };
            // 内置：打开 Kite 设置
            if item.id == "kite:settings" {
                return open_settings(state);
            }
            // 内置：切换文件搜索模式
            if item.id == "kite:action:files" {
                state.files_mode = !state.files_mode;
                if !state.files_mode {
                    state.file_filter = system::everything::FileFilter::All;
                }
                state.request_file_search();
                state.refresh_results();
                return sync_scroll(state);
            }
            // 非内联工具：选中「JSON 工具」后 Enter 打开（列表无取消行）。
            if item.id == super::json_tool::CONFIRM_RESULT_ID {
                return super::tools::confirm_open(state);
            }
            if let Some(url) = everything_download_url(&item.id) {
                let result = app::uwp::launch_shell_path(url);
                state.qlog(|| format!("open Everything download err={result:?}"));
                return if result.is_ok() {
                    launch_and_hide(state)
                } else {
                    flash(state, "无法打开 Everything 官方下载页")
                };
            }
            if item.id == system::everything::NOT_RUNNING_RESULT_ID {
                return flash(state, "请先启动 Everything，再使用文件搜索");
            }
            if let Some((kind, browser_id, payload)) = app::web::parse_id(&item.id) {
                let t0 = Instant::now();
                system::env::refresh_process_env();
                let cached = state.history.as_ref().and_then(|h| h.search_url_template());
                let result = if kind == "websearch" {
                    app::web::launch_websearch(&browser_id, &payload, cached.as_deref())
                } else {
                    app::web::launch_url(&browser_id, &payload)
                };
                return match result {
                    Ok(preferred) => {
                        let elapsed_us = t0.elapsed().as_micros();
                        state.qlog(|| {
                            format!("launch web {kind} via {browser_id} in {elapsed_us}us")
                        });
                        if let Some(db) = &mut state.history {
                            let q = search::normalize_for_index(&state.query);
                            let _ = db.record_launch(&item.id, &q, storage::now_ts());
                            if let Some(pid) = preferred {
                                let _ = db.set_preferred_browser(&pid);
                            }
                        }
                        state.invalidate_prefs_cache();
                        launch_and_hide(state)
                    }
                    Err(e) => {
                        state.qlog(|| format!("launch web {kind} failed: {e}"));
                        Task::none()
                    }
                };
            }
            let t0 = Instant::now();
            system::env::refresh_process_env();
            match app::launch_with_terminal(&item, state.cli_terminal) {
                Ok(()) => {
                    let elapsed_us = t0.elapsed().as_micros();
                    let (name, target) = (item.display_name.clone(), item.target.clone());
                    state.qlog(|| format!("launch '{name}' target={target} in {elapsed_us}us ok"));
                    if let Some(db) = &mut state.history {
                        let q = search::normalize_for_index(&state.query);
                        let _ = db.record_launch(&item.id, &q, storage::now_ts());
                    }
                    state.invalidate_prefs_cache();
                    launch_and_hide(state)
                }
                Err(e) => {
                    state.qlog(|| {
                        format!(
                            "launch '{}' target={} failed: {e}",
                            item.display_name, item.target
                        )
                    });
                    flash(state, &format!("启动失败：{e}"))
                }
            }
        }
    }
}

fn exec_direct_path(state: &mut State, path: &str, reveal: bool) -> Task<Message> {
    let Some(candidate) = app::actions::direct_path_candidate(path) else {
        return flash(state, "路径无效");
    };
    let Ok(metadata) = std::fs::metadata(&candidate) else {
        return flash(state, "路径不存在或无法访问");
    };
    if !metadata.is_file() && !metadata.is_dir() {
        return flash(state, "路径不是文件或文件夹");
    }
    let operation = if reveal {
        app::actions::open_containing_folder(path)
    } else {
        app::uwp::launch_shell_path(path)
    };
    match operation {
        Ok(()) => launch_and_hide(state),
        Err(error) => {
            state.qlog(|| format!("direct path action failed: {error}"));
            flash(state, "无法打开该路径")
        }
    }
}

pub(super) fn exec_native_panel_action(
    state: &mut State,
    native: plugin::panel::NativeAction,
) -> Task<Message> {
    use plugin::panel::NativeAction;
    match native {
        NativeAction::CopyText(text) => {
            state.qlog(|| "panel native copy_text".to_owned());
            hide(state);
            Task::batch([iced::clipboard::write(text), hide_task(state)])
        }
        NativeAction::OpenUrl(url) => open_url_and_hide(state, &url),
        NativeAction::OpenPath(path) => open_path_and_hide(state, &path),
    }
}

fn exec_plugin_action(
    state: &mut State,
    plugin_id: &str,
    action_id: &str,
    payload: serde_json::Value,
) -> Task<Message> {
    if action_id == "enter_trigger_provider" {
        let query = state.query.clone();
        let activation =
            super::with_plugin_registry(state, |reg| plugin::route_query(&query, reg));
        let Some(act) = activation.filter(|act| {
            act.plugin_id == plugin_id
                && payload.get("provider").and_then(|v| v.as_str())
                    == Some(act.provider_id.as_str())
        }) else {
            return Task::none();
        };
        if super::json_tool::is_native_json_activation(&act) {
            let payload = act.effective_query.trim().to_string();
            let task = super::tools::arm_confirm(
                state,
                super::tools::ToolKind::Json,
                (!payload.is_empty()).then_some(payload),
                false,
            );
            return Task::batch([task, super::tools::confirm_open(state)]);
        }
        if super::hash_tool::is_native_hash_activation(&act) {
            return super::tools::open_tool_panel(
                state,
                super::tools::ToolKind::Hash,
                Some(act.effective_query),
                true,
            );
        }
        if super::base64_tool::is_native_base64_activation(&act) {
            return super::tools::open_tool_panel(
                state,
                super::tools::ToolKind::Base64,
                Some(act.effective_query),
                true,
            );
        }
        state.provider_mode = Some(act);
        state.plugin_panel = None;
        state.refresh_results();
        return focus(search_view::input_id());
    }
    // Command → enter_provider（List/Panel 共用：写入 Trigger 前缀后 refresh 路由）
    if action_id == "enter_provider" {
        let resolved = {
            let command_id = payload
                .get("command_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let provider = payload
                .get("provider")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            super::with_plugin_registry(state, |reg| {
                plugin::activation::resolve_command_entry(
                    reg,
                    plugin_id,
                    &command_id,
                    provider.as_deref(),
                )
            })
        };
        if let Some((act, initial_query)) = resolved {
            if super::hash_tool::is_native_hash_activation(&act) {
                return super::tools::open_tool_panel(
                    state,
                    super::tools::ToolKind::Hash,
                    None,
                    true,
                );
            }
            if super::base64_tool::is_native_base64_activation(&act) {
                return super::tools::open_tool_panel(
                    state,
                    super::tools::ToolKind::Base64,
                    None,
                    true,
                );
            }
            // 非内联 json 工具：二次确认，不直接开窗；内联 Provider 照旧。
            if super::json_tool::is_native_json_activation(&act) {
                state.query = initial_query;
                state.results.clear();
                state.results_stale = false;
                state.selected = 0;
                state.navigation_mode = NavigationMode::Input;
                state.provider_mode = None;
                state.plugin_panel = None;
                let payload = act.effective_query.trim().to_string();
                return super::tools::arm_confirm(
                    state,
                    super::tools::ToolKind::Json,
                    if payload.is_empty() {
                        None
                    } else {
                        Some(payload)
                    },
                    false,
                );
            }
            state.provider_mode = Some(act.clone());
            state.plugin_panel = None;
            state.navigation_mode = NavigationMode::Input;
            state.plugin_query_generation = state.plugin_query_generation.wrapping_add(1);
            state.plugin_query_worker.cancel_current();
            state.query = initial_query;
            state.qlog(|| {
                format!(
                    "command enter_provider {:?} q={:?}",
                    act.provider_id, state.query
                )
            });
            state.refresh_results();
        }
        return focus(search_view::input_id());
    }

    // 其它 PluginAction → plugin/execute（异步，不阻塞 UI 线程）
    let registry = state.plugin_registry.clone();
    let host = state.plugin_host.clone();
    let pid = plugin_id.to_string();
    let aid = action_id.to_string();
    std::thread::spawn(move || {
        let host_calls = {
            let reg = registry.lock().unwrap_or_else(|e| e.into_inner());
            let mut host = host.lock().unwrap_or_else(|e| e.into_inner());
            let _ = host.execute(&reg, &pid, &aid, payload);
            host.drain_host_calls()
        };
        for (pid, call) in host_calls {
            super::send_event(Message::PluginHostCall(pid, call));
        }
    });
    Task::none()
}

fn everything_download_url(item_id: &str) -> Option<&'static str> {
    (item_id == system::everything::DOWNLOAD_RESULT_ID).then_some(system::everything::DOWNLOAD_URL)
}

#[cfg(test)]
mod everything_action_tests {
    use super::*;

    #[test]
    fn missing_dependency_result_targets_official_download_page() {
        assert_eq!(
            everything_download_url(system::everything::DOWNLOAD_RESULT_ID),
            Some("https://www.voidtools.com/downloads/")
        );
        assert_eq!(
            everything_download_url(system::everything::NOT_RUNNING_RESULT_ID),
            None
        );
    }
}

#[cfg(test)]
mod interactive_log_gate_tests {
    use super::super::test_support::test_state;
    use super::*;
    use crate::log;
    use crate::ui::interaction::update;
    use iced::keyboard::key::Code;

    fn press(state: &mut State, key: Key) -> Task<Message> {
        on_key(state, key, Physical::Code(Code::KeyA), Modifiers::empty())
    }

    /// 关掉查询日志后，交互路径（Esc、未识别具名键、Alt+数字、启动、右键菜单入口、
    /// 文件模式切换、设置页开合）不得产生任何日志行 —— 不调用 `plog`，
    /// 也就没有同步文件写入。
    #[test]
    fn interactive_path_writes_no_log_when_query_log_off() {
        let _guard = crate::ui::backend::PENDING_FULL_TEST_LOCK.lock().unwrap();
        let mut state = test_state("k");
        state.query_log = false;

        log::test_records();
        // Esc：隐藏启动器
        let _ = press(&mut state, Key::Named(Named::Escape));
        // 未识别的具名键
        let _ = press(&mut state, Key::Named(Named::Tab));
        // 启动：结果过期时被拒
        state.hidden = false;
        state.results_stale = true;
        let _ = launch_selected(&mut state);
        // 启动：内置「Kite 设置」→ 设置页打开，再关掉
        state.results_stale = false;
        let _ = launch_selected(&mut state);
        let _ = close_settings(&mut state);
        // Alt+数字
        state.hidden = false;
        state.alt_digit_consumed = false;
        let _ = on_key(
            &mut state,
            Key::Character("1".into()),
            Physical::Code(Code::Digit1),
            Modifiers::ALT,
        );
        // 文件模式切换与文件搜索落地／过期
        let _ = update(&mut state, Message::ToggleFiles);
        let _ = update(
            &mut state,
            Message::FileSearchReady(0, String::new(), Vec::new(), 0),
        );
        // 设置窗口内的动作：快捷键变更被拒
        let _ = update(
            &mut state,
            Message::HotkeyRegistrationResult("Alt+X".into(), Err("boom".into())),
        );

        assert_eq!(
            log::test_records(),
            Vec::<String>::new(),
            "关闭查询日志后，交互路径不得产生任何同步日志写入"
        );
    }

    /// 开关开启（默认）时，同样的交互路径照常写日志 —— 证明上面的断言确实覆盖了这些点。
    #[test]
    fn interactive_path_still_logs_when_query_log_on() {
        let mut state = test_state("k");
        assert!(state.query_log, "查询日志默认开启");

        log::test_records();
        let _ = press(&mut state, Key::Named(Named::Escape));
        let _ = update(&mut state, Message::ToggleFiles);
        let _ = update(
            &mut state,
            Message::FileSearchReady(0, String::new(), Vec::new(), 0),
        );
        let _ = update(
            &mut state,
            Message::HotkeyRegistrationResult("Alt+X".into(), Err("boom".into())),
        );

        let records = log::test_records();
        assert!(
            records.iter().any(|r| r.contains("hide issued (esc)")),
            "开启时 Esc 必须仍写日志，实际：{records:?}"
        );
        assert!(
            records.iter().any(|r| r.contains("files toggle ->")),
            "开启时文件模式切换必须仍写日志，实际：{records:?}"
        );
        assert!(
            records.iter().any(|r| r.contains("file search ")),
            "开启时文件搜索落地必须仍写日志，实际：{records:?}"
        );
        assert!(
            records.iter().any(|r| r.contains("hotkey change rejected")),
            "开启时设置窗口动作必须仍写日志，实际：{records:?}"
        );
    }
}

/// 把逻辑尺寸窗口摆到光标所在显示器工作区中心（物理定位 → 按窗口当前 scale 转逻辑）。
pub(super) fn place_on_cursor_monitor_task(
    id: window::Id,
    window_w: f32,
    window_h: f32,
) -> Task<Message> {
    window::scale_factor(id).then(move |scale| {
        let Some((px, py)) =
            system::window_place::physical_position_on_cursor_monitor(window_w, window_h)
        else {
            return Task::none();
        };
        let (lx, ly) = system::window_place::physical_to_logical_for_window((px, py), scale);
        window::move_to(id, iced::Point::new(lx, ly))
    })
}

/// 显示主窗口（热键/托盘/二次启动共用）。窗口未就绪时忽略。
pub(super) fn show_launcher(state: &mut State) -> Task<Message> {
    let Some(id) = state.window_id else {
        plog("show before window ready; ignored");
        return Task::none();
    };
    // 下次打开时采用挂起的 Full。
    super::interaction::apply_pending_full(state);
    // 热键/托盘唤起只还原主启动器搜索；JSON 工具窗独立，不随唤起关闭。
    state.settings_open = false;
    state.plugin_docs_open = None;
    state.navigation_mode = NavigationMode::Input;
    state.hidden = false;
    state.epoch += 1;
    // 每次唤起按鼠标所在 monitor 重新定位（uTools 式多显示器跟随）。
    let place = system::window_place::physical_position_on_cursor_monitor(WINDOW_W, WINDOW_H);
    if state.files_mode {
        state.request_file_search();
    }
    state.refresh_results();
    // 对齐 Kite：唤起即响（SND_ASYNC，不阻塞显示）
    system::sound::play_open();
    plog(&format!(
        "show issued epoch={} place={place:?}",
        state.epoch
    ));
    Task::batch([
        // 搜索窗口只需要前台焦点，不应持续置顶，否则会压住截图层和其它全局快捷键 UI。
        window::set_level(id, window::Level::Normal),
        // gain_focus 在窗口不可见时是 no-op：必须先 set_mode 再 focus，不能 batch 并行。
        window::set_mode(id, window::Mode::Windowed)
            .chain(window::gain_focus(id))
            .chain(iced::widget::operation::focus(state.input_id.clone())),
        sync_scroll(state),
        place_on_cursor_monitor_task(id, WINDOW_W, WINDOW_H),
    ])
}

pub(super) fn hide(state: &mut State) {
    state.hidden = true;
    state.navigation_mode = NavigationMode::Input;
    state.ime_composing = false;
    state.alt_down = false;
    state.query_at_alt = None;
    state.alt_digit_consumed = false;
    close_menu(state);
    state.query.clear();
    state.request_direct_path();
    state.invalidate_file_search();
    // 隐藏后采用后台 Full，用户下次看到的就是新快照。
    super::interaction::apply_pending_full(state);
    state.refresh_results();
}

pub(super) fn hide_task(state: &State) -> Task<Message> {
    state
        .window_id
        .map(|id| window::set_mode(id, window::Mode::Hidden))
        .unwrap_or_else(Task::none)
}

/// 启动成功后的统一收尾：埋点日志 + 隐藏主窗。
pub(super) fn launch_and_hide(state: &mut State) -> Task<Message> {
    state.qlog(|| "hide issued (launch)".to_owned());
    hide(state);
    hide_task(state)
}

/// 校验并打开 URL，成功则隐藏主窗。
pub(super) fn open_url_and_hide(state: &mut State, url: &str) -> Task<Message> {
    if !plugin::plugin_url_allowed(url) {
        state.qlog(|| "open_url rejected non-http(s)".to_owned());
        return flash(state, "链接无效");
    }
    system::env::refresh_process_env();
    match app::uwp::launch_shell_path(url) {
        Ok(()) => launch_and_hide(state),
        Err(e) => {
            state.qlog(|| format!("open_url failed: {e}"));
            flash(state, "无法打开链接")
        }
    }
}

/// 校验并打开路径，成功则隐藏主窗。
pub(super) fn open_path_and_hide(state: &mut State, path: &str) -> Task<Message> {
    if !plugin::plugin_path_allowed(path) {
        state.qlog(|| "open_path rejected".to_owned());
        return flash(state, "路径无效");
    }
    system::env::refresh_process_env();
    match app::uwp::launch_shell_path(path) {
        Ok(()) => launch_and_hide(state),
        Err(e) => {
            state.qlog(|| format!("open_path failed: {e}"));
            flash(state, "无法打开该路径")
        }
    }
}

/// 设置页提示条（css settings-toast），2s 自动消失。
pub(super) fn flash(state: &mut State, msg: &str) -> Task<Message> {
    state.flash = Some(msg.to_string());
    Task::perform(
        async {
            std::thread::sleep(std::time::Duration::from_secs(2));
        },
        |()| Message::FlashClear,
    )
}

/// 打开设置：载入设置与别名，窗口切到 720×520（对齐 set_settings_mode）。
pub(super) fn open_settings(state: &mut State) -> Task<Message> {
    state.settings_open = true;
    // 设置在主窗口打开；JSON 工具窗独立，不在此关闭。
    state.hidden = false;
    state.settings_section = Section::General;
    close_menu(state);
    if let Some(db) = &state.history {
        let s = db.load_settings();
        state.hide_on_blur = s.hide_on_blur;
        state.autostart = s.autostart;
        state.history_recording = s.history_recording;
        state.query_log = s.query_log;
        state.cli_terminal = app::CliTerminal::from_setting(&s.cli_terminal);
        state.search_engine = s.search_engine.clone();
        state.web_search_hotkey = s.web_search_hotkey.clone();
        state.search_engine_custom = db
            .search_url_template()
            .or_else(|| {
                system::search_engine::preset_by_id(&state.search_engine)
                    .filter(|p| !p.template.is_empty())
                    .map(|p| p.template.to_string())
            })
            .unwrap_or_default();
        state.hotkey = s.hotkey.clone();
        state.hotkey_label = s.hotkey_label.clone();
        state.portable_dirs = s.portable_dirs;
        let mut options = state
            .scan_options
            .write()
            .unwrap_or_else(|error| error.into_inner());
        options.portable_dirs = state
            .portable_dirs
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
    }
    load_aliases(state);
    load_hidden(state);
    load_manual_apps(state);
    sync_manual_apps_to_scan_options(state);
    state.qlog(|| "settings open".to_owned());
    let Some(id) = state.window_id else {
        return Task::none();
    };
    settings_window_task(id)
        // 先切换到设置尺寸，再按该尺寸居中，避免窗口任务并行时互相覆盖位置。
        .chain(place_on_cursor_monitor_task(id, SETTINGS_W, SETTINGS_H))
}

const SETTINGS_W: f32 = 720.0;
const SETTINGS_H: f32 = 520.0;

pub(super) fn settings_window_task(id: window::Id) -> Task<Message> {
    // gain_focus 在窗口不可见时是 no-op：必须先 set_mode/resize 再 focus，不能 batch 并行。
    window::set_level(id, window::Level::Normal)
        .chain(window::set_mode(id, window::Mode::Windowed))
        .chain(window::resize(id, iced::Size::new(SETTINGS_W, SETTINGS_H)))
        .chain(window::gain_focus(id))
}

/// 关闭设置：恢复搜索尺寸和居中位置，再聚焦输入框。
pub(super) fn close_settings(state: &mut State) -> Task<Message> {
    state.settings_open = false;
    state.flash = None;
    state.plugin_docs_open = None;
    super::interaction::apply_pending_full(state);
    state.refresh_results();
    // 工具面板是独立形态，不随设置关闭而打开。
    state.qlog(|| "settings close".to_owned());
    let Some(id) = state.window_id else {
        return Task::none();
    };
    window::set_level(id, window::Level::Normal)
        .chain(window::resize(id, iced::Size::new(WINDOW_W, WINDOW_H)))
        .chain(place_on_cursor_monitor_task(id, WINDOW_W, WINDOW_H))
        .chain(iced::widget::operation::focus(state.input_id.clone()))
}

pub(super) fn load_aliases(state: &mut State) {
    if let Some(db) = &state.history {
        if let Ok(list) = db.list_aliases() {
            state.aliases = list;
        }
    }
}

/// 别名增删改后的统一失效入口。
///
/// 别名目标直接决定搜索里明确匹配保护层的命中，因此基础候选缓存以及正在跑的
/// 请求都要作废：否则同一个输入会继续按旧别名排序；而只清空缓存也不够——
/// 在途任务完成时还会把用旧别名算出的候选写回来（由缓存代际拦住）。
pub(super) fn refresh_after_alias_change(state: &mut State) {
    state.base_hit_cache.invalidate();
    state.refresh_results();
}

/// 热键录制态：下一组按键即新快捷键（Esc 取消）。
pub(super) fn hotkey_record_key(state: &mut State, key: Key, mods: Modifiers) -> Task<Message> {
    if let Key::Named(Named::Escape) = key {
        super::update_settings::end_hotkey_record(state);
        return flash(state, "已取消");
    }
    let key_name: Option<String> = match &key {
        Key::Character(c) => c
            .chars()
            .next()
            .map(|ch| ch.to_ascii_uppercase().to_string()),
        Key::Named(n) => match n {
            Named::Space => Some("Space".into()),
            Named::Tab => Some("Tab".into()),
            Named::F1 => Some("F1".into()),
            Named::F2 => Some("F2".into()),
            Named::F3 => Some("F3".into()),
            Named::F4 => Some("F4".into()),
            Named::F5 => Some("F5".into()),
            Named::F6 => Some("F6".into()),
            Named::F7 => Some("F7".into()),
            Named::F8 => Some("F8".into()),
            Named::F9 => Some("F9".into()),
            Named::F10 => Some("F10".into()),
            Named::F11 => Some("F11".into()),
            Named::F12 => Some("F12".into()),
            _ => None,
        },
        _ => None,
    };
    let Some(k) = key_name else {
        return Task::none();
    };
    let mut parts: Vec<&str> = Vec::new();
    if mods.control() {
        parts.push("Ctrl");
    }
    if mods.alt() {
        parts.push("Alt");
    }
    if mods.shift() {
        parts.push("Shift");
    }
    if mods.logo() {
        parts.push("Win");
    }
    if parts.is_empty() {
        return flash(state, "请连同修饰键一起按下，例如 Ctrl+Alt+K");
    }
    let spec = {
        let mut s = parts.join("+");
        s.push('+');
        s.push_str(&k);
        s
    };
    super::update_settings::end_hotkey_record(state);
    Task::done(Message::ApplyHotkey(spec))
}

#[cfg(test)]
mod result_panel_tests {
    use super::*;
    use crate::ui::test_support::{settings_result, test_state};
    use crate::{model::SearchResult, storage::HistoryDb};
    use iced::keyboard::key::Code;

    fn temp_db(tag: &str) -> HistoryDb {
        let path = std::env::temp_dir().join(format!(
            "kite-ui-{tag}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        HistoryDb::open(&path).expect("test history db")
    }

    fn app(id: &str) -> AppItem {
        let mut item = AppItem::scanned(
            id.into(),
            format!("App {id}"),
            format!(r"C:\Apps\{id}.exe"),
            None,
            None,
            "manual",
        );
        item.attach_search_fields();
        item
    }

    #[test]
    fn shift_f10_opens_shared_panel_and_captures_query() {
        let mut state = test_state("Visual");
        state.hidden = false;
        let _ = on_key(
            &mut state,
            Key::Named(Named::F10),
            Physical::Code(Code::F10),
            Modifiers::SHIFT,
        );
        let (item, _, _) = state.menu.clone().expect("Shift+F10 opens panel");
        assert_eq!(item.id, "kite:settings");
        assert_eq!(state.menu_query, "Visual");
        assert_eq!(state.menu_selected, 0);
    }

    #[test]
    fn panel_keeps_captured_target_when_results_reorder() {
        let mut state = test_state("k");
        state.hidden = false;
        let mut second = state.results[0].clone();
        second.item.id = "kite:other".into();
        state.results.push(second);
        let _ = open_context_menu(&mut state, 0);
        let captured = state.menu.as_ref().map(|(item, _, _)| item.id.clone());
        state.results.reverse();
        assert_eq!(captured.as_deref(), Some("kite:settings"));
        assert_eq!(
            state.menu.as_ref().map(|(item, _, _)| item.id.as_str()),
            Some("kite:settings")
        );
    }

    #[test]
    fn ime_composition_blocks_panel_enter() {
        let mut state = test_state("k");
        state.hidden = false;
        let _ = open_context_menu(&mut state, 0);
        state.ime_composing = true;
        let _ = on_key(
            &mut state,
            Key::Named(Named::Enter),
            Physical::Code(Code::Enter),
            Modifiers::empty(),
        );
        assert!(state.menu.is_some(), "IME Enter must not execute or close panel");
    }

    #[test]
    fn alt_text_fallback_cannot_launch_while_panel_is_open() {
        let mut state = test_state("k");
        state.hidden = false;
        let _ = open_context_menu(&mut state, 0);
        state.alt_down = true;
        state.query_at_alt = Some("k".into());
        let _ = super::super::update_search::handle(
            &mut state,
            Message::QueryChanged("k1".into()),
        );
        assert!(state.menu.is_some(), "Alt text fallback must not close the panel");
        assert!(!state.settings_open, "Alt text fallback must not launch the target");
    }

    #[test]
    fn alias_action_reports_conflict_before_replacing_existing_mapping() {
        let mut state = test_state("vs");
        state.hidden = false;
        state.history = Some(temp_db("alias"));
        let item = app("new");
        let other = app("old");
        state.index.lock().unwrap().apps = vec![item.clone(), other.clone()];
        state.results = vec![SearchResult::scored(item.clone(), 1000, "exact")];
        state
            .history
            .as_mut()
            .unwrap()
            .set_alias("vs", Some(&other.id), &other.display_name)
            .unwrap();
        load_aliases(&mut state);
        let _ = open_context_menu(&mut state, 0);
        let _ = menu_action(&mut state, item, MenuAction::SetAlias);
        state.action_alias_input = "vs".into();
        let _ = super::super::update_search::handle(&mut state, Message::ActionAliasSave);
        assert!(state.action_alias_conflict.is_some());
        assert_eq!(
            state.history.as_ref().unwrap().alias_matches("vs")[0]
                .target_id
                .as_deref(),
            Some("old")
        );
    }

    #[test]
    fn changing_alias_text_invalidates_old_conflict_confirmation() {
        let mut state = test_state("vs");
        state.hidden = false;
        state.history = Some(temp_db("alias-edit"));
        let item = app("new-edit");
        let other = app("old-edit");
        state.index.lock().unwrap().apps = vec![item.clone(), other.clone()];
        state.results = vec![SearchResult::scored(item.clone(), 1000, "exact")];
        state
            .history
            .as_mut()
            .unwrap()
            .set_alias("vs", Some(&other.id), &other.display_name)
            .unwrap();
        load_aliases(&mut state);
        let _ = open_context_menu(&mut state, 0);
        let _ = menu_action(&mut state, item, MenuAction::SetAlias);
        let _ = super::super::update_search::handle(&mut state, Message::ActionAliasSave);
        assert!(state.action_alias_conflict.is_some());
        let _ = super::super::update_search::handle(
            &mut state,
            Message::ActionAliasInputChanged("another".into()),
        );
        let _ = super::super::update_search::handle(&mut state, Message::ActionAliasReplace);
        assert!(state.action_alias_conflict.is_none());
        assert_eq!(
            state.history.as_ref().unwrap().alias_matches("vs")[0]
                .target_id
                .as_deref(),
            Some(other.id.as_str())
        );
    }

    #[test]
    fn baseline_pin_actions_remain_for_plugin_and_system_entries() {
        let mut state = test_state("");
        state.hidden = false;
        state.history = Some(temp_db("baseline-actions"));
        let mut plugin = app("plugin-entry");
        plugin.source = "plugin".into();
        let mut builtin = app("builtin-entry");
        builtin.source = "builtin".into();
        state.index.lock().unwrap().system_entries = vec![builtin.clone()];

        let plugin_entries = menu_entries(&state, &plugin);
        assert!(plugin_entries
            .iter()
            .any(|(_, action)| *action == MenuAction::TogglePin));
        assert!(!plugin_entries
            .iter()
            .any(|(_, action)| *action == MenuAction::SetAlias));
        let builtin_entries = menu_entries(&state, &builtin);
        assert!(builtin_entries
            .iter()
            .any(|(_, action)| *action == MenuAction::TogglePin));
        let _ = menu_action(&mut state, builtin, MenuAction::TogglePin);
        assert!(state
            .history
            .as_ref()
            .unwrap()
            .pinned_ids()
            .contains(&"builtin-entry".to_string()));

        // Kite's own settings/tools are synthesized command results rather
        // than members of either scanned index. Their trusted IDs must still
        // retain the baseline pin action without a stale-index lookup.
        let settings = settings_result().item;
        assert!(menu_entries(&state, &settings)
            .iter()
            .any(|(_, action)| *action == MenuAction::TogglePin));
        let _ = menu_action(&mut state, settings, MenuAction::TogglePin);
        assert!(state
            .history
            .as_ref()
            .unwrap()
            .pinned_ids()
            .contains(&"kite:settings".to_string()));
    }

    #[test]
    fn forget_query_uses_panel_snapshot_and_preserves_other_query() {
        let mut state = test_state(" VS ");
        state.hidden = false;
        state.history = Some(temp_db("forget"));
        state
            .history
            .as_mut()
            .unwrap()
            .record_launch("app", "vs", 1)
            .unwrap();
        state
            .history
            .as_mut()
            .unwrap()
            .record_launch("app", "code", 2)
            .unwrap();
        let item = app("app");
        state.results = vec![SearchResult::scored(item.clone(), 1000, "exact")];
        let _ = open_context_menu(&mut state, 0);
        state.query = "later".into();
        let _ = menu_action(&mut state, item, MenuAction::ForgetQuery);
        assert!(state.history.as_ref().unwrap().query_pairs_for("vs").is_empty());
        assert!(!state.history.as_ref().unwrap().query_pairs_for("code").is_empty());
    }
}
