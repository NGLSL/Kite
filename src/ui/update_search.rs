//! 查询输入、结果选择与后台搜索结果落地。

use super::actions::{
    close_menu, flash, launch_alt_digit, launch_selected, menu_action, open_context_menu,
    sync_scroll,
};
use super::interaction::{apply_pending_full, request_rescan};
use super::results;
use super::{alt_digit_from_query_change, Message, NavigationMode, State};
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
            state.qlog(|| {
                format!(
                    "file search ready generation={generation} query={query:?} hits={} elapsed={elapsed_us}us",
                    hits.len()
                )
            });
            state.file_results = hits;
            state.refresh_results();
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
