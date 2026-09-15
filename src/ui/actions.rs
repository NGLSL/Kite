//! 搜索窗口的选择、启动、设置和快捷键动作。

use super::*;

/// 上下文菜单动作。
pub(super) fn menu_action(state: &mut State, item: AppItem, action: MenuAction) -> Task<Message> {
    state.menu = None;
    match action {
        MenuAction::OpenFolder => {
            let r = app::actions::open_containing_folder(&item.target);
            plog(&format!("ctx open_folder err={r:?}"));
        }
        MenuAction::CopyPath => return iced::clipboard::write(item.target),
        MenuAction::CopyName => return iced::clipboard::write(item.display_name),
        MenuAction::TogglePin => {
            if let Some(db) = &mut state.history {
                let r = if state.pinned.contains(&item.id) {
                    db.unpin_item(&item.id)
                } else {
                    db.pin_item(&item.id, storage::now_ts())
                };
                plog(&format!("ctx pin toggle ok={}", r.is_ok()));
            }
            state.refresh_results();
        }
    }
    Task::none()
}

/// 键盘：↑↓ 选择、Enter 启动（组合态禁止，见 ime_composing）、Esc 关菜单/隐藏、
/// Alt+1..9 启动对应行（对齐前端 index<9 提示）；热键录制态优先捕获。
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

    // Alt+1..9：兼容 modifiers.alt() / 本地 alt_down / 逻辑字符 / 物理 Digit|Numpad
    let alt_idx = alt_digit_index(&key, physical, mods, state.alt_down);
    if let Some(i) = alt_idx {
        plog(&format!(
            "alt-n idx={i} alt_down={} mods_alt={} key={key:?} phys={physical:?}",
            state.alt_down,
            mods.alt()
        ));
        return launch_alt_digit(state, i);
    }

    match key {
        Key::Named(Named::Escape) if !state.ime_composing => {
            // 前端行为：菜单开着时 Esc 只关菜单；设置页开着时 Esc 回搜索
            if state.menu.take().is_some() {
                plog("ctx menu closed (esc)");
                return Task::none();
            }
            if state.settings_open {
                return close_settings(state);
            }
            plog("hide issued (esc)");
            hide(state);
            hide_task(state)
        }
        Key::Named(Named::ArrowUp) => move_selection(state, -1),
        Key::Named(Named::ArrowDown) => move_selection(state, 1),
        Key::Named(Named::Enter) if !state.ime_composing => launch_selected(state),
        Key::Named(Named::Alt) => Task::none(),
        Key::Named(name) => {
            plog(&format!("key named {name:?} ignored"));
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
        }
    }
    state.selected = i;
    launch_selected(state)
}

/// 选中行滚入可视区（保留上方两行），对应前端滚动定位行为。
pub(super) fn sync_scroll(state: &State) -> Task<Message> {
    let y =
        ((state.selected as f32) * search_view::ROW_STEP - 2.0 * search_view::ROW_STEP).max(0.0);
    scroll_to(search_view::scroll_id(), AbsoluteOffset { x: 0.0, y })
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

pub(super) fn launch_selected(state: &mut State) -> Task<Message> {
    let Some(item) = state.results.get(state.selected).map(|r| r.item.clone()) else {
        plog("enter with empty results; ignored");
        return Task::none();
    };
    state.menu = None;

    // 内置：打开 Kite 设置
    if item.id == "kite:settings" {
        return open_settings(state);
    }

    if let Some(url) = everything_download_url(&item.id) {
        let result = app::uwp::launch_shell_path(url);
        plog(&format!("open Everything download err={result:?}"));
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

    // 浏览器打开网址 / 网页搜索（id 由 app::web 生成，对齐 commands::launch_app）
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
                plog(&format!(
                    "launch web {kind} via {browser_id} in {}us",
                    t0.elapsed().as_micros()
                ));
                if let Some(db) = &mut state.history {
                    let q = search::normalize_for_index(&state.query);
                    let _ = db.record_launch(&item.id, &q, storage::now_ts());
                    if let Some(pid) = preferred {
                        let _ = db.set_preferred_browser(&pid);
                    }
                }
                plog("hide issued (launch)");
                hide(state);
                hide_task(state)
            }
            Err(e) => {
                plog(&format!("launch web {kind} failed: {e}"));
                Task::none()
            }
        };
    }

    let t0 = Instant::now();
    // 对齐 commands::launch_app：先刷新进程环境再拉起
    system::env::refresh_process_env();
    match app::launch(&item) {
        Ok(()) => {
            plog(&format!(
                "launch '{}' target={} in {}us ok",
                item.display_name,
                item.target,
                t0.elapsed().as_micros()
            ));
            if let Some(db) = &mut state.history {
                let q = search::normalize_for_index(&state.query);
                let _ = db.record_launch(&item.id, &q, storage::now_ts());
            }
            plog("hide issued (launch)");
            hide(state);
            hide_task(state)
        }
        Err(e) => {
            plog(&format!(
                "launch '{}' target={} failed: {e}",
                item.display_name, item.target
            ));
            Task::none()
        }
    }
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

pub(super) fn hide(state: &mut State) {
    state.hidden = true;
    state.ime_composing = false;
    state.alt_down = false;
    state.query_at_alt = None;
    state.alt_digit_consumed = false;
    state.menu = None;
    state.query.clear();
    state.invalidate_file_search();
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
    state.hidden = false;
    state.settings_section = Section::General;
    state.menu = None;
    if let Some(db) = &state.history {
        let s = db.load_settings();
        state.hide_on_blur = s.hide_on_blur;
        state.autostart = s.autostart;
        state.history_recording = s.history_recording;
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
    plog("settings open");
    state
        .window_id
        .map(settings_window_task)
        .unwrap_or_else(Task::none)
}

pub(super) fn settings_window_task(id: window::Id) -> Task<Message> {
    Task::batch([
        window::set_level(id, window::Level::Normal),
        window::set_mode(id, window::Mode::Windowed),
        window::resize(id, iced::Size::new(720.0, 520.0)),
        window::gain_focus(id),
    ])
}

/// 关闭设置：窗口切回搜索尺寸并聚焦输入框。
pub(super) fn close_settings(state: &mut State) -> Task<Message> {
    state.settings_open = false;
    state.flash = None;
    plog("settings close");
    Task::batch([
        state
            .window_id
            .map(|id| window::set_level(id, window::Level::Normal))
            .unwrap_or_else(Task::none),
        state
            .window_id
            .map(|id| window::resize(id, iced::Size::new(WINDOW_W, WINDOW_H)))
            .unwrap_or_else(Task::none),
        iced::widget::operation::focus(state.input_id.clone()),
    ])
}

pub(super) fn load_aliases(state: &mut State) {
    if let Some(db) = &state.history {
        if let Ok(list) = db.list_aliases() {
            state.aliases = list;
        }
    }
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
