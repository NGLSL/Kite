//! 查询输入、结果选择与后台搜索结果落地。

use super::actions::{flash, launch_alt_digit, launch_selected, menu_action, sync_scroll};
use super::interaction::{capture_menu_item, request_rescan};
use super::results;
use super::{alt_digit_from_query_change, Message, NavigationMode, State};
use iced::Task;

pub(super) fn handle(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::QueryChanged(q) => {
            if state.hidden {
                return Task::none();
            }
            if state.alt_down && !state.hotkey_recording && !state.ime_composing {
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
            state.menu = None;
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
                let n = if std::path::Path::new(&item.target).is_file() {
                    4
                } else {
                    2
                };
                let h = 8.0 + n as f32 * 33.0;
                if c.x < *x || c.x > *x + 180.0 || c.y < *y || c.y > *y + h {
                    state.menu = None;
                }
            }
            Task::none()
        }
        Message::ContextMenu(i) => {
            if state.menu.is_some() {
                state.menu = None;
                state.qlog(|| "ctx menu closed (re-right-click)".to_owned());
                return Task::none();
            }
            let c = state.cursor.get();
            let x = c.x.min(640.0 - 200.0).max(8.0);
            let y = c.y.min(420.0 - 180.0).max(8.0);
            state.menu =
                capture_menu_item(&state.results, i).map(|item| (item, x, y));
            Task::none()
        }
        Message::MenuAction(item, action) => menu_action(state, item, action),
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
