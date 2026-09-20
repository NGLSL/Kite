//! 搜索窗口的选择、启动、设置和快捷键动作。

use super::*;

/// 上下文菜单动作。
pub(super) fn menu_action(state: &mut State, item: AppItem, action: MenuAction) -> Task<Message> {
    state.menu = None;
    match action {
        MenuAction::OpenFolder => {
            let r = app::actions::open_containing_folder(&item.target);
            state.qlog(|| format!("ctx open_folder err={r:?}"));
        }
        MenuAction::CopyPath => return iced::clipboard::write(item.target),
        MenuAction::CopyTarget => {
            let t = item.target.trim();
            let body = t
                .strip_prefix("shell:AppsFolder\\")
                .or_else(|| t.strip_prefix("shell:appsfolder\\"))
                .unwrap_or(t);
            return iced::clipboard::write(body.to_string());
        }
        MenuAction::CopyName => return iced::clipboard::write(item.display_name),
        MenuAction::TogglePin => {
            if let Some(db) = &mut state.history {
                let r = if state.pinned.contains(&item.id) {
                    db.unpin_item(&item.id)
                } else {
                    db.pin_item(&item.id, storage::now_ts())
                };
                state.qlog(|| format!("ctx pin toggle ok={}", r.is_ok()));
            }
            // 与 Demote 共用同一入口：非空 Query 同样交给常驻 worker 重排。
            state.refresh_results();
        }
        MenuAction::Demote | MenuAction::Undemote => {
            if let Some(db) = &mut state.history {
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
            }
            state.refresh_results();
        }
    }
    Task::none()
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
    // Settings and the standalone JSON tool own their text/button keyboard
    // interaction. Keep only Escape global so captured widget events cannot
    // move or launch a result in the hidden/secondary UI.
    if (state.settings_open || state.plugin_tool_open) && !matches!(key, Key::Named(Named::Escape))
    {
        return Task::none();
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
            if state.menu.take().is_some() {
                state.qlog(|| "ctx menu closed (esc)".to_owned());
                return Task::none();
            }
            // JSON 工具窗：Esc 关闭工具窗，主窗不动。
            if state.plugin_tool_open {
                return if state.base64_tool_window.is_some() {
                    close_base64_tool_panel(state)
                } else if state.hash_tool_window.is_some() {
                    close_hash_tool_panel(state)
                } else {
                    close_json_tool_panel(state)
                };
            }
            // 二次确认：Esc 取消，不开工具窗。
            if state.pending_tool_confirm.is_some() {
                return cancel_json_tool_confirm(state);
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
            hide(state);
            hide_task(state)
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
    state.menu = None;
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
                    hide(state);
                    hide_task(state)
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
                    hide(state);
                    hide_task(state)
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
                return confirm_json_tool_open(state);
            }
            if let Some(url) = everything_download_url(&item.id) {
                let result = app::uwp::launch_shell_path(url);
                state.qlog(|| format!("open Everything download err={result:?}"));
                return if result.is_ok() {
                    hide(state);
                    hide_task(state)
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
                        state.qlog(|| "hide issued (launch)".to_owned());
                        hide(state);
                        hide_task(state)
                    }
                    Err(e) => {
                        state.qlog(|| format!("launch web {kind} failed: {e}"));
                        Task::none()
                    }
                };
            }
            let t0 = Instant::now();
            system::env::refresh_process_env();
            match app::launch(&item) {
                Ok(()) => {
                    let elapsed_us = t0.elapsed().as_micros();
                    let (name, target) = (item.display_name.clone(), item.target.clone());
                    state.qlog(|| format!("launch '{name}' target={target} in {elapsed_us}us ok"));
                    if let Some(db) = &mut state.history {
                        let q = search::normalize_for_index(&state.query);
                        let _ = db.record_launch(&item.id, &q, storage::now_ts());
                    }
                    state.qlog(|| "hide issued (launch)".to_owned());
                    hide(state);
                    hide_task(state)
                }
                Err(e) => {
                    state.qlog(|| {
                        format!(
                            "launch '{}' target={} failed: {e}",
                            item.display_name, item.target
                        )
                    });
                    Task::none()
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
        Ok(()) => {
            hide(state);
            hide_task(state)
        }
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
        NativeAction::OpenUrl(url) => {
            if !plugin::plugin_url_allowed(&url) {
                state.qlog(|| "panel open_url rejected non-http(s)".to_owned());
                return flash(state, "链接无效");
            }
            system::env::refresh_process_env();
            let r = app::uwp::launch_shell_path(&url);
            if r.is_ok() {
                hide(state);
                hide_task(state)
            } else {
                flash(state, "无法打开链接")
            }
        }
        NativeAction::OpenPath(path) => {
            if !plugin::plugin_path_allowed(&path) {
                state.qlog(|| "panel open_path rejected".to_owned());
                return flash(state, "路径无效");
            }
            system::env::refresh_process_env();
            let r = app::uwp::launch_shell_path(&path);
            if r.is_ok() {
                hide(state);
                hide_task(state)
            } else {
                flash(state, "无法打开该路径")
            }
        }
    }
}

fn exec_plugin_action(
    state: &mut State,
    plugin_id: &str,
    action_id: &str,
    payload: serde_json::Value,
) -> Task<Message> {
    if action_id == "enter_trigger_provider" {
        let activation = {
            let reg = state
                .plugin_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            plugin::route_query(&state.query, &reg)
        };
        let Some(act) = activation.filter(|act| {
            act.plugin_id == plugin_id
                && payload.get("provider").and_then(|v| v.as_str())
                    == Some(act.provider_id.as_str())
        }) else {
            return Task::none();
        };
        if super::json_tool::is_native_json_activation(&act) {
            let payload = act.effective_query.trim().to_string();
            let task =
                arm_json_tool_confirm(state, (!payload.is_empty()).then_some(payload), false);
            return Task::batch([task, confirm_json_tool_open(state)]);
        }
        if super::hash_tool::is_native_hash_activation(&act) {
            return open_hash_tool_panel(state, Some(act.effective_query));
        }
        if super::base64_tool::is_native_base64_activation(&act) {
            return open_base64_tool_panel(state, Some(act.effective_query), true);
        }
        state.provider_mode = Some(act);
        state.plugin_panel = None;
        state.refresh_results();
        return focus(search_view::input_id());
    }
    // Command → enter_provider（List/Panel 共用：写入 Trigger 前缀后 refresh 路由）
    if action_id == "enter_provider" {
        let resolved = {
            let reg = state
                .plugin_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let command_id = payload
                .get("command_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let provider = payload
                .get("provider")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            plugin::activation::resolve_command_entry(
                &reg,
                plugin_id,
                &command_id,
                provider.as_deref(),
            )
        };
        if let Some((act, initial_query)) = resolved {
            if super::hash_tool::is_native_hash_activation(&act) {
                return open_hash_tool_panel(state, None);
            }
            if super::base64_tool::is_native_base64_activation(&act) {
                return open_base64_tool_panel(state, None, true);
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
                return arm_json_tool_confirm(
                    state,
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
        if let Some(tx) = super::EVENT_TX.get() {
            for (pid, call) in host_calls {
                let _ = tx.unbounded_send(Message::PluginHostCall(pid, call));
            }
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
fn place_on_cursor_monitor_task(id: window::Id, window_w: f32, window_h: f32) -> Task<Message> {
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
    // 下次打开时采用挂起的 Full，避免可见期间打断列表。
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
    state.menu = None;
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
    state.menu = None;
    if let Some(db) = &state.history {
        let s = db.load_settings();
        state.hide_on_blur = s.hide_on_blur;
        state.autostart = s.autostart;
        state.history_recording = s.history_recording;
        state.query_log = s.query_log;
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

/// 进入打开 JSON 工具的二次确认（不直接开窗）。
pub(super) fn open_json_tool_panel(state: &mut State) -> Task<Message> {
    arm_json_tool_confirm(state, None, true)
}

/// 非内联工具（JSON 独立窗）打开前二次确认。内联插件不走这里。
pub(super) fn arm_json_tool_confirm(
    state: &mut State,
    payload: Option<String>,
    from_settings: bool,
) -> Task<Message> {
    state.pending_tool_confirm = Some(PendingJsonToolConfirm {
        payload: payload.filter(|p| !p.trim().is_empty()),
        from_settings,
    });
    state.qlog(|| "json tool confirm armed".to_owned());
    if from_settings {
        // 设置页内联确认条，不改主窗。
        return Task::none();
    }
    // 搜索列表：只展示一条「JSON 工具」，Enter 打开；Esc 取消，不另出取消行。
    state.results = json_tool_confirm_results(state.pending_tool_confirm.as_ref());
    state.results_stale = false;
    state.selected = 0;
    state.navigation_mode = NavigationMode::Input;
    state.hover_suppressed = false;
    state.provider_mode = None;
    state.plugin_panel = None;
    sync_scroll(state)
}

/// 确认后真正打开 JSON 独立工具窗。
pub(super) fn confirm_json_tool_open(state: &mut State) -> Task<Message> {
    let Some(pending) = state.pending_tool_confirm.take() else {
        return Task::none();
    };
    open_json_tool_panel_with_payload(state, pending.payload, !pending.from_settings)
}

/// 取消二次确认：不开工具窗；搜索态恢复当前查询结果。
pub(super) fn cancel_json_tool_confirm(state: &mut State) -> Task<Message> {
    let from_settings = state
        .pending_tool_confirm
        .as_ref()
        .map(|p| p.from_settings)
        .unwrap_or(true);
    state.pending_tool_confirm = None;
    state.qlog(|| "json tool confirm cancelled".to_owned());
    if from_settings {
        return Task::none();
    }
    state.refresh_results();
    sync_scroll(state)
}

/// 搜索列表结果：只展示「JSON 工具」一条；Enter 打开，Esc 取消（无取消行）。
fn json_tool_confirm_results(pending: Option<&PendingJsonToolConfirm>) -> Vec<SearchResult> {
    let has_payload = pending
        .and_then(|p| p.payload.as_deref())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let sub = if has_payload {
        "独立工具窗 · 预填格式化 · Enter 打开"
    } else {
        "独立工具窗 · Enter 打开"
    };
    vec![SearchResult::scored(
        AppItem::scanned(
            super::json_tool::CONFIRM_RESULT_ID.into(),
            "JSON 工具".into(),
            sub.into(),
            None,
            None,
            "builtin",
        ),
        980,
        "builtin",
    )]
}

/// 打开 JSON 工具窗；`payload` 非空时预填左侧并自动格式化。
/// `from_search`：从搜索确认打开时隐藏主窗（内容/尺寸不变）；设置确认则主窗保持原样。
pub(super) fn open_json_tool_panel_with_payload(
    state: &mut State,
    payload: Option<String>,
    from_search: bool,
) -> Task<Message> {
    match payload {
        Some(raw) if !raw.trim().is_empty() => {
            state.json_editor = iced::widget::text_editor::Content::with_text(&raw);
            match super::json_tool::transform_json(&raw, false) {
                Ok(out) => {
                    state.json_result = out;
                    state.json_tool_note = Some((false, "已格式化".into()));
                }
                Err(msg) => {
                    state.json_result.clear();
                    state.json_tool_note = Some((true, msg));
                }
            }
        }
        Some(_) | None => {
            // 空 payload / 直接打开：保留用户上次编辑，不强行清空。
        }
    }

    // 工具窗已存在：只更新内容并抢焦点，不重复开窗。
    if let Some(tid) = state.json_tool_window {
        state.qlog(|| "json tool focus".to_owned());
        return window::gain_focus(tid);
    }

    let (w, h) = (super::json_tool::TOOL_W, super::json_tool::TOOL_H);
    let (tool_id, open_task) = window::open(Settings {
        size: iced::Size::new(w, h),
        position: Position::SpecificWith(|win, monitor| {
            iced::Point::new(
                (monitor.width - win.width) / 2.0,
                (monitor.height - win.height) / 2.0,
            )
        }),
        visible: true,
        resizable: false,
        decorations: false,
        level: window::Level::Normal,
        exit_on_close_request: false,
        platform_specific: PlatformSpecific {
            skip_taskbar: false,
            undecorated_shadow: false,
            corner_preference: iced::window::settings::platform::CornerPreference::Round,
            ..PlatformSpecific::default()
        },
        ..Settings::default()
    });
    state.json_tool_window = Some(tool_id);
    state.plugin_tool_open = true;
    state.qlog(|| "json tool open".to_owned());

    let mut tasks = vec![open_task.then(move |_opened| {
        Task::batch([
            window::gain_focus(tool_id),
            place_on_cursor_monitor_task(tool_id, w, h),
        ])
    })];
    if from_search {
        // 搜索触发：主启动器仅隐藏，不切换视图、不改尺寸。
        if let Some(main) = state.window_id {
            state.hidden = true;
            tasks.push(window::set_mode(main, window::Mode::Hidden));
        }
    }
    Task::batch(tasks)
}

/// 关闭 JSON 工具窗：只销毁工具窗，主启动器窗口尺寸/内容不动。
pub(super) fn close_json_tool_panel(state: &mut State) -> Task<Message> {
    let Some(tid) = state.json_tool_window else {
        state.plugin_tool_open =
            state.hash_tool_window.is_some() || state.base64_tool_window.is_some();
        return Task::none();
    };
    state.json_tool_window = None;
    state.plugin_tool_open =
        state.hash_tool_window.is_some() || state.base64_tool_window.is_some();
    state.qlog(|| "json tool close".to_owned());
    window::close(tid)
}

/// 打开 Hash 独立窗；从搜索进入时只隐藏启动器，保留其查询和结果。
pub(super) fn open_hash_tool_panel(state: &mut State, payload: Option<String>) -> Task<Message> {
    if let Some(raw) = payload.filter(|s| !s.is_empty()) {
        state.hash_editor = iced::widget::text_editor::Content::with_text(&raw);
        state.hash_result = hash_tool::sha256_hex(&raw);
        state.hash_tool_note = None;
    }
    if let Some(id) = state.hash_tool_window {
        return window::gain_focus(id);
    }
    let (w, h) = (hash_tool::TOOL_W, hash_tool::TOOL_H);
    let (id, opened) = window::open(Settings {
        size: iced::Size::new(w, h),
        position: Position::SpecificWith(|win, monitor| {
            iced::Point::new(
                (monitor.width - win.width) / 2.0,
                (monitor.height - win.height) / 2.0,
            )
        }),
        visible: true,
        resizable: false,
        decorations: false,
        level: window::Level::Normal,
        exit_on_close_request: false,
        platform_specific: PlatformSpecific {
            skip_taskbar: false,
            undecorated_shadow: false,
            corner_preference: iced::window::settings::platform::CornerPreference::Round,
            ..PlatformSpecific::default()
        },
        ..Settings::default()
    });
    state.hash_tool_window = Some(id);
    state.plugin_tool_open = true;
    state.qlog(|| "hash tool open".to_owned());
    let mut tasks = vec![opened.then(move |_opened| {
        Task::batch([
            window::gain_focus(id),
            place_on_cursor_monitor_task(id, w, h),
        ])
    })];
    if let Some(main) = state.window_id {
        state.hidden = true;
        tasks.push(window::set_mode(main, window::Mode::Hidden));
    }
    Task::batch(tasks)
}

pub(super) fn close_hash_tool_panel(state: &mut State) -> Task<Message> {
    let Some(id) = state.hash_tool_window.take() else {
        return Task::none();
    };
    state.plugin_tool_open =
        state.json_tool_window.is_some() || state.base64_tool_window.is_some();
    state.qlog(|| "hash tool close".to_owned());
    window::close(id)
}

/// 打开 Base64 独立窗；搜索触发时隐藏启动器，设置页触发时保留设置页。
pub(super) fn open_base64_tool_panel(
    state: &mut State,
    payload: Option<String>,
    from_search: bool,
) -> Task<Message> {
    if let Some(raw) = payload.filter(|s| !s.is_empty()) {
        state.base64_editor = iced::widget::text_editor::Content::with_text(&raw);
        state.base64_result = base64_tool::encode_utf8(&raw);
        state.base64_tool_note = Some((false, "已编码".into()));
    }
    if let Some(id) = state.base64_tool_window {
        return window::gain_focus(id);
    }
    let (w, h) = (base64_tool::TOOL_W, base64_tool::TOOL_H);
    let (id, opened) = window::open(Settings {
        size: iced::Size::new(w, h),
        position: Position::SpecificWith(|win, monitor| {
            iced::Point::new(
                (monitor.width - win.width) / 2.0,
                (monitor.height - win.height) / 2.0,
            )
        }),
        visible: true,
        resizable: false,
        decorations: false,
        level: window::Level::Normal,
        exit_on_close_request: false,
        platform_specific: PlatformSpecific {
            skip_taskbar: false,
            undecorated_shadow: false,
            corner_preference: iced::window::settings::platform::CornerPreference::Round,
            ..PlatformSpecific::default()
        },
        ..Settings::default()
    });
    state.base64_tool_window = Some(id);
    state.plugin_tool_open = true;
    state.qlog(|| "base64 tool open".to_owned());
    let mut tasks = vec![opened.then(move |_opened| {
        Task::batch([
            window::gain_focus(id),
            place_on_cursor_monitor_task(id, w, h),
        ])
    })];
    if from_search {
        if let Some(main) = state.window_id {
            state.hidden = true;
            tasks.push(window::set_mode(main, window::Mode::Hidden));
        }
    }
    Task::batch(tasks)
}

pub(super) fn close_base64_tool_panel(state: &mut State) -> Task<Message> {
    let Some(id) = state.base64_tool_window.take() else {
        return Task::none();
    };
    state.plugin_tool_open = state.json_tool_window.is_some() || state.hash_tool_window.is_some();
    state.qlog(|| "base64 tool close".to_owned());
    window::close(id)
}

/// 关闭设置：恢复搜索尺寸和居中位置，再聚焦输入框。
pub(super) fn close_settings(state: &mut State) -> Task<Message> {
    state.settings_open = false;
    state.flash = None;
    state.plugin_docs_open = None;
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
        state.hotkey_recording = false;
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
    state.hotkey_recording = false;
    Task::done(Message::ApplyHotkey(spec))
}
