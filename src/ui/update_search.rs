//! 查询输入、结果选择与后台搜索结果落地。

use super::actions::{
    close_menu, flash, launch_alt_digit, launch_selected, menu_action, open_context_menu,
    sync_scroll,
};
use super::interaction::{apply_pending_full, request_rescan};
use super::results;
use super::{alt_digit_from_query_change, Message, NavigationMode, Section, State};
use iced::Task;

pub(super) fn handle(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::QueryChanged(q) => {
            if state.hidden {
                return Task::none();
            }
            if state.action_alias_item.is_some() {
                return Task::none();
            }
            if state.alt_down
                && state.menu.is_none()
                && !state.hotkey_recording
                && !state.ime_composing
            {
                if let Some(i) = state
                    .query_at_alt
                    .as_deref()
                    .and_then(|before| alt_digit_from_query_change(before, &q))
                {
                    if state.alt_digit_consumed {
                        return Task::none();
                    }
                    state.qlog(|| format!("alt-digit text fallback idx={i}"));
                    return launch_alt_digit(state, i);
                }
            }
            state.query = q;
            // Full 可能在窗口可见时已完成；输入新查询前采用它，避免继续查旧索引。
            apply_pending_full(state);
            state.request_direct_path();
            state.navigation_mode = NavigationMode::Input;
            if state
                .pending_tool_confirm
                .as_ref()
                .is_some_and(|p| !p.from_settings)
            {
                state.pending_tool_confirm = None;
            }
            state.request_file_search();
            state.refresh_results();
            state.hover_suppressed = false;
            Task::batch([
                sync_scroll(state),
                iced::widget::operation::focus(state.input_id.clone()),
            ])
        }
        Message::ClearQuery => {
            state.query.clear();
            state.request_direct_path();
            state.navigation_mode = NavigationMode::Input;
            state.request_file_search();
            state.refresh_results();
            Task::batch([
                iced::widget::operation::focus(state.input_id.clone()),
                sync_scroll(state),
            ])
        }
        Message::HoverSelect(i) => {
            if state.hover_suppressed {
                return Task::none();
            }
            if state.selected != i {
                state.selected = i;
            }
            state.navigation_mode = NavigationMode::Results;
            Task::none()
        }
        Message::LaunchIndex(i) => {
            close_menu(state);
            state.selected = i;
            state.navigation_mode = NavigationMode::Results;
            launch_selected(state)
        }
        Message::ToggleFiles => {
            state.files_mode = !state.files_mode;
            if !state.files_mode {
                state.file_filter = crate::system::everything::FileFilter::All;
                state.file_search_confirm_open = false;
            }
            state.navigation_mode = NavigationMode::Input;
            state.qlog(|| format!("files toggle -> {}", state.files_mode));
            state.request_file_search();
            state.refresh_results();
            Task::batch([
                sync_scroll(state),
                iced::widget::operation::focus(state.input_id.clone()),
            ])
        }
        Message::FileSearchEnableConfirm => {
            if let Err(error) = save_file_search_enabled(state, true) {
                state.file_search_confirm_open = true;
                return flash(state, &format!("文件搜索设置保存失败：{error}"));
            }
            state.file_search_confirm_open = false;
            state.file_search_enabled = true;
            state.file_search_controller.start(true);
            state.invalidate_file_search();
            state.request_file_search();
            state.refresh_results();
            Task::batch([
                flash(state, "正在请求授权并建立文件索引…"),
                sync_scroll(state),
            ])
        }
        Message::FileSearchEnableCancel => {
            state.file_search_confirm_open = false;
            Task::none()
        }
        Message::FileSearchSetEnabled(enabled) => {
            if enabled {
                if state.file_search_enabled {
                    if state.file_search_controller.snapshot().phase
                        == crate::file_search::Phase::NeedsAuthorization
                    {
                        // 已保存启用但服务尚未获权时，复用首次授权确认卡；
                        // 真正的 start(true) 仍只发生在用户确认之后。
                        state.file_search_confirm_open = true;
                    }
                    return Task::none();
                }
                state.file_search_confirm_open = true;
                if state.settings_open {
                    // The plugin list is only an entry point. Keep the first-use
                    // confirmation and subsequent progress on the dedicated page.
                    state.settings_section = Section::FileSearch;
                }
                return Task::none();
            }
            if let Err(error) = save_file_search_enabled(state, false) {
                return flash(state, &format!("文件搜索设置保存失败：{error}"));
            }
            state.file_search_confirm_open = false;
            state.file_search_enabled = false;
            state.file_search_controller.stop();
            state.invalidate_file_search();
            state.files_mode = false;
            state.file_filter = crate::system::everything::FileFilter::All;
            state.refresh_results();
            flash(state, "文件搜索已关闭")
        }
        Message::FileSearchRetry => {
            let phase = state.file_search_controller.snapshot().phase;
            if matches!(
                phase,
                crate::file_search::Phase::Starting
                    | crate::file_search::Phase::Indexing
                    | crate::file_search::Phase::CatchingUp
            ) {
                return flash(state, "文件索引正在建立，请等待当前任务完成");
            }
            state.file_search_controller.start(false);
            if state.files_mode {
                state.invalidate_file_search();
                state.request_file_search();
                state.refresh_results();
            }
            flash(state, "正在重试文件索引连接…")
        }
        Message::FileSearchRebuild => {
            if !state.file_search_enabled {
                state.file_search_confirm_open = true;
                return Task::none();
            }
            if matches!(
                state.file_search_controller.snapshot().phase,
                crate::file_search::Phase::Starting
                    | crate::file_search::Phase::Indexing
                    | crate::file_search::Phase::CatchingUp
            ) {
                return flash(state, "文件索引正在建立，请等待当前任务完成");
            }
            state.file_search_controller.rebuild(false);
            state.invalidate_file_search();
            state.request_file_search();
            state.refresh_results();
            flash(state, "已开始重建文件索引…")
        }
        Message::FileSearchExcludeInputChanged(value) => {
            state.file_search_exclusion_input = value;
            Task::none()
        }
        Message::FileSearchExcludeAdd => add_file_search_exclusion(state),
        Message::FileSearchExcludeRemove(index) => remove_file_search_exclusion(state, index),
        Message::FileSearchStatusTick => {
            let snapshot = state.file_search_controller.snapshot();
            let changed = snapshot.revision != state.file_search_snapshot.revision
                || snapshot.phase != state.file_search_snapshot.phase
                || snapshot.entries != state.file_search_snapshot.entries
                || snapshot.detail != state.file_search_snapshot.detail;
            if !changed {
                return Task::none();
            }
            let was_ready = state.file_search_snapshot.phase == crate::file_search::Phase::Ready;
            let is_ready = snapshot.phase == crate::file_search::Phase::Ready;
            let revision_changed = snapshot.revision != state.file_search_snapshot.revision;
            state.file_search_snapshot = snapshot;
            // Snapshot 继续在后台更新；仅主搜索窗口可见时才消费 metadata
            // revision 做查询/状态重绘，避免隐藏窗口因 USN 变化持续扫全库。
            let visible_main_search = state.files_mode
                && !state.hidden
                && !state.settings_open
                && !state.any_tool_open()
                && state.provider_mode.is_none();
            if visible_main_search {
                if is_ready && !was_ready {
                    // 采集/追平完成是阶段边界，丢弃阶段中的状态结果并开始
                    // 当前 query；这和用户修改 query 一样需要严格失效旧代际。
                    state.request_file_search();
                    state.refresh_results();
                } else if is_ready && revision_changed {
                    // Ready 后的增量 revision 不改变 query 代际，也不清空已
                    // 显示的命中。若查询在途只记 dirty，返回后补一次。
                    state.request_file_search_refresh();
                } else if was_ready && !is_ready {
                    // 索引离开 Ready 时，之前的命中不再代表当前可查询状态；
                    // 先使在途响应失效，再展示读取/构建/追平状态。
                    state.invalidate_file_search();
                    refresh_file_status_result(state);
                } else {
                    // 采集中只更新轻量状态行，让“读取/构建/追平”进度即时
                    // 可见，不触碰正在显示的文件命中。
                    refresh_file_status_result(state);
                }
            }
            Task::none()
        }
        Message::FileFilterChanged(filter) => {
            if !state.files_mode || state.file_filter == filter {
                return Task::none();
            }
            state.file_filter = filter;
            state.navigation_mode = NavigationMode::Input;
            state.request_file_search();
            state.refresh_results();
            Task::batch([
                sync_scroll(state),
                iced::widget::operation::focus(state.input_id.clone()),
            ])
        }
        Message::FileSearchReady(generation, query, hits, elapsed_us) => {
            if !results::is_current_file_response(
                state.files_mode,
                state.file_query_generation,
                &state.query,
                generation,
                &query,
            ) {
                state.qlog(|| {
                    format!(
                        "file search stale generation={generation} query={query:?} elapsed={elapsed_us}us"
                    )
                });
                return Task::none();
            }
            state.file_query_in_flight = false;
            state.qlog(|| {
                format!(
                    "file search ready generation={generation} query={query:?} hits={} elapsed={elapsed_us}us",
                    hits.len()
                )
            });
            state.file_results = hits;
            state.refresh_results();
            let revision_changed =
                state.file_search_controller.snapshot().revision != state.file_query_revision;
            let refresh_needed = state.file_query_dirty || revision_changed;
            state.file_query_dirty = false;
            if refresh_needed {
                // 当前结果已经可见；只补一个同代际查询，不走严格失效，
                // 这样连续 USN 变化也不会造成列表饥饿或无限排队。
                state.request_file_search_refresh();
            }
            Task::none()
        }
        Message::DirectPathReady(generation, query, hits) => {
            if generation != state.direct_path_generation || query != state.query {
                return Task::none();
            }
            state.direct_path_results = hits;
            state.refresh_results();
            Task::none()
        }
        Message::AppSearchReady(generation, query, hits, elapsed_us, index_generation) => {
            state.apply_app_search_ready(generation, query, hits, elapsed_us, index_generation);
            sync_scroll(state)
        }
        Message::CursorMoved(p) => {
            state.cursor.set(p);
            let moved = match state.last_hover_pt {
                Some(last) => (last.x - p.x).abs() > 1.0 || (last.y - p.y).abs() > 1.0,
                None => true,
            };
            if moved {
                state.hover_suppressed = false;
                state.last_hover_pt = Some(p);
            }
            Task::none()
        }
        Message::MousePressed(btn) => {
            if btn != iced::mouse::Button::Left {
                return Task::none();
            }
            if let Some((item, x, y)) = &state.menu {
                let c = state.cursor.get();
                let h = super::actions::menu_height(state, item);
                if c.x < *x || c.x > *x + 240.0 || c.y < *y || c.y > *y + h {
                    // Widget focus traverses all windows; do not unfocus a
                    // standalone tool editor when dismissing the main menu.
                    let restore_input =
                        state.action_alias_item.is_some() && !state.any_tool_open();
                    close_menu(state);
                    if restore_input {
                        return iced::widget::operation::focus(super::search_view::input_id());
                    }
                }
            }
            Task::none()
        }
        Message::ContextMenu(i) => {
            state.qlog(|| "ctx menu requested".to_owned());
            open_context_menu(state, i)
        }
        Message::MenuAction(item, action) => menu_action(state, item, action),
        Message::ActionAliasInputChanged(value) => {
            state.action_alias_input = value;
            // A conflict confirmation belongs to the exact alias text that
            // produced it. Editing the text invalidates that confirmation.
            state.action_alias_conflict = None;
            Task::none()
        }
        Message::ActionAliasSave => handle_action_alias_save(state),
        Message::ActionAliasReplace => handle_action_alias_replace(state),
        Message::ActionAliasCancel => {
            super::actions::cancel_action_alias(state)
        }
        Message::Rescan => {
            state.qlog(|| "rescan requested from settings/tray".to_owned());
            state.rescan_pending = true;
            {
                let mut options = state
                    .scan_options
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                options.force_uwp_refresh = true;
            }
            request_rescan(state);
            flash(state, "正在重新扫描应用索引…")
        }
        _ => Task::none(),
    }
}

fn add_file_search_exclusion(state: &mut State) -> Task<Message> {
    let path = match crate::storage::settings::normalize_file_search_exclusion(
        &state.file_search_exclusion_input,
    ) {
        Ok(path) => path,
        Err(error) => return flash(state, &error),
    };
    let key = path.to_ascii_lowercase();
    if state
        .file_search_exclusions
        .iter()
        .any(|item| item.to_ascii_lowercase() == key)
    {
        return flash(state, "该文件夹已经在排除列表中");
    }
    state.file_search_exclusions.push(path);
    if let Err(error) = persist_file_search_exclusions(state) {
        state.file_search_exclusions.pop();
        return flash(state, &format!("排除文件夹保存失败：{error}"));
    }
    state.file_search_exclusion_input.clear();
    state.invalidate_file_search();
    state.request_file_search();
    state.refresh_results();
    flash(state, "排除文件夹已立即生效")
}

fn remove_file_search_exclusion(state: &mut State, index: usize) -> Task<Message> {
    if index >= state.file_search_exclusions.len() {
        return Task::none();
    }
    let removed = state.file_search_exclusions.remove(index);
    if let Err(error) = persist_file_search_exclusions(state) {
        state.file_search_exclusions.insert(index, removed);
        return flash(state, &format!("排除文件夹保存失败：{error}"));
    }
    state.invalidate_file_search();
    state.request_file_search();
    state.refresh_results();
    flash(state, "已取消排除，文件搜索结果已更新")
}

fn persist_file_search_exclusions(state: &mut State) -> Result<(), String> {
    let saved = if let Some(db) = &mut state.history {
        db.set_file_search_exclude_dirs(&state.file_search_exclusions)?
    } else {
        state.file_search_exclusions.clone()
    };
    state.file_search_exclusions = saved.clone();
    state.file_search_controller.set_exclusions(saved);
    Ok(())
}

fn save_file_search_enabled(state: &mut State, enabled: bool) -> Result<(), String> {
    let Some(db) = &mut state.history else {
        return Ok(());
    };
    db.save_setting("file_search_enabled", if enabled { "1" } else { "0" })
        .map_err(|error| error.to_string())
}

fn refresh_file_status_result(state: &mut State) {
    state.file_results = super::results::build_indexed_file_results(
        &state.query,
        &state.icon_dir,
        state.file_filter,
        &state.file_search_controller,
        state.file_search_enabled,
    );
    let status = state.file_results.iter().find(|result| {
        result.item.source == "file-search-status" || result.item.source == "everything-status"
    });
    state.results.retain(|result| {
        result.item.source != "file-search-status" && result.item.source != "everything-status"
    });
    if let Some(status) = status.cloned() {
        state.results.insert(0, status);
    }
}

fn handle_action_alias_save(state: &mut State) -> Task<Message> {
    if state.ime_composing {
        return Task::none();
    }
    let alias = state.action_alias_input.trim().to_lowercase();
    if alias.is_empty() {
        return flash(state, "请填写 Alias");
    }
    let Some(item) = state.action_alias_item.clone() else {
        return Task::none();
    };
    let Some(current) = super::actions::resolve_current_item(state, &item) else {
        return flash(state, "目标已不在当前索引或启动身份已变化，未保存 Alias");
    };
    if !super::actions::is_app_entry(&current) {
        return flash(state, "当前目标不支持 Alias");
    }
    if let Some(existing) = state
        .aliases
        .iter()
        .find(|candidate| candidate.alias == alias)
        .cloned()
    {
        if existing.target_id.as_deref() != Some(item.id.as_str()) {
            state.action_alias_conflict = Some(existing);
            super::actions::clamp_menu_height(state, 220.0);
            return flash(state, "Alias 已存在，请确认是否替换");
        }
    }
    save_action_alias(state, &current, &alias)
}

fn handle_action_alias_replace(state: &mut State) -> Task<Message> {
    if state.ime_composing {
        return Task::none();
    }
    let Some(item) = state.action_alias_item.clone() else {
        return Task::none();
    };
    let Some(conflict) = state.action_alias_conflict.clone() else {
        return flash(state, "请先确认 Alias 冲突");
    };
    let alias = state.action_alias_input.trim().to_lowercase();
    if alias.is_empty() {
        return flash(state, "请填写 Alias");
    }
    let still_same_conflict = state.aliases.iter().any(|existing| {
        existing.alias == conflict.alias
            && existing.alias == alias
            && existing.target_id == conflict.target_id
            && existing.target_name == conflict.target_name
    });
    if !still_same_conflict {
        state.action_alias_conflict = None;
        return flash(state, "Alias 冲突已变化，请重新保存确认");
    }
    let Some(current) = super::actions::resolve_current_item(state, &item) else {
        return flash(state, "目标已不在当前索引或启动身份已变化，未替换 Alias");
    };
    if !super::actions::is_app_entry(&current) {
        return flash(state, "当前目标不支持 Alias");
    }
    save_action_alias(state, &current, &alias)
}

fn save_action_alias(
    state: &mut State,
    item: &crate::model::AppItem,
    alias: &str,
) -> Task<Message> {
    let Some(db) = state.history.as_mut() else {
        return flash(state, "历史库不可用，无法保存 Alias");
    };
    if let Err(error) = db.set_alias(alias, Some(&item.id), &item.display_name) {
        return flash(state, &format!("Alias 保存失败：{error}"));
    }
    super::actions::load_aliases(state);
    super::actions::refresh_after_alias_change(state);
    super::actions::close_menu(state);
    Task::batch([
        flash(state, "Alias 已保存"),
        iced::widget::operation::focus(super::search_view::input_id()),
    ])
}
