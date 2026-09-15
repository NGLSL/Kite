//! 键盘事件分类与 Alt+数字键识别。

use super::{Message, State};
use iced::keyboard::{
    self,
    key::{Code, Named, Physical},
    Key, Modifiers,
};
use iced::{window, Subscription};

pub(super) fn keyboard_events(state: &State) -> Subscription<Message> {
    // 正常运行时只订阅 Kite 自己使用的按键；否则外部截图、录屏和辅助工具的
    // 单键快捷键会先进入 Iced 窗口。录制快捷键时才临时接收完整按键流。
    if state.hotkey_recording {
        iced::event::listen().filter_map(keyboard_message_recording)
    } else {
        iced::event::listen().filter_map(keyboard_message_app)
    }
}

fn keyboard_message_recording(event: iced::event::Event) -> Option<Message> {
    keyboard_message(event, true)
}

fn keyboard_message_app(event: iced::event::Event) -> Option<Message> {
    keyboard_message(event, false)
}

fn keyboard_message(event: iced::event::Event, recording: bool) -> Option<Message> {
    match event {
        iced::event::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if recording || app_key(&key, physical_key, modifiers) => {
            Some(Message::KeyPressed(key, physical_key, modifiers))
        }
        iced::event::Event::Keyboard(keyboard::Event::KeyReleased { key, modifiers, .. })
            if recording || matches!(key, Key::Named(Named::Alt)) =>
        {
            Some(Message::KeyReleased(key, modifiers))
        }
        iced::event::Event::InputMethod(im) => match im {
            iced_core::input_method::Event::Opened => Some(Message::Composing(true)),
            iced_core::input_method::Event::Preedit(s, _) => {
                Some(Message::Composing(!s.is_empty()))
            }
            iced_core::input_method::Event::Commit(s) => Some(Message::ImeCommit(s)),
            iced_core::input_method::Event::Closed => Some(Message::Composing(false)),
        },
        iced::event::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::CursorMoved(position))
        }
        iced::event::Event::Window(window::Event::Unfocused) => Some(Message::WindowBlur),
        iced::event::Event::Mouse(iced::mouse::Event::ButtonPressed(btn)) => {
            Some(Message::MousePressed(btn))
        }
        _ => None,
    }
}

fn app_key(key: &Key, physical: Physical, mods: Modifiers) -> bool {
    match key {
        Key::Named(
            Named::Escape | Named::ArrowUp | Named::ArrowDown | Named::Enter | Named::Alt,
        ) => true,
        Key::Character(c) => {
            // 字符路径：带 Alt 的数字，或任意字符（后续再判，避免 SYSKEY 下 modifiers 丢 Alt）
            let _ = c;
            true
        }
        _ => {
            // 物理数字键（含小键盘）：无 Alt 也放行，由 on_key 用本地 alt_down / modifiers 判定
            physical_digit_code(physical).is_some() || mods.alt()
        }
    }
}

fn physical_digit_code(physical: Physical) -> Option<u32> {
    let code = match physical {
        Physical::Code(code) => code,
        Physical::Unidentified(_) => return None,
    };
    match code {
        Code::Digit1 | Code::Numpad1 => Some(1),
        Code::Digit2 | Code::Numpad2 => Some(2),
        Code::Digit3 | Code::Numpad3 => Some(3),
        Code::Digit4 | Code::Numpad4 => Some(4),
        Code::Digit5 | Code::Numpad5 => Some(5),
        Code::Digit6 | Code::Numpad6 => Some(6),
        Code::Digit7 | Code::Numpad7 => Some(7),
        Code::Digit8 | Code::Numpad8 => Some(8),
        Code::Digit9 | Code::Numpad9 => Some(9),
        _ => None,
    }
}

/// Alt+1..9 → 结果下标。
/// 兼容：iced modifiers.alt()、本地跟踪的 Alt 按下态、逻辑字符、物理 Digit/Numpad。
pub(super) fn alt_digit_index(
    key: &Key,
    physical: Physical,
    mods: Modifiers,
    alt_down: bool,
) -> Option<usize> {
    if !mods.alt() && !alt_down {
        return None;
    }
    let from_char = |c: &str| {
        c.chars()
            .next()
            .and_then(|ch| ch.to_digit(10))
            .filter(|d| *d >= 1)
            .map(|d| (d - 1) as usize)
    };
    if let Key::Character(c) = key {
        if let Some(i) = from_char(c) {
            return Some(i);
        }
    }
    physical_digit_code(physical).map(|d| (d - 1) as usize)
}

/// 某些 Windows 输入链路只向 text_input 报告 Alt+数字后的文本变化，
/// 不向应用订阅交付数字 KeyPressed。只接受在按下 Alt 时 Query 后追加的单个 1..9。
pub(super) fn alt_digit_from_query_change(before: &str, after: &str) -> Option<usize> {
    let appended = after.strip_prefix(before)?;
    if appended.len() != 1 {
        return None;
    }
    appended
        .chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .filter(|d| (1..=9).contains(d))
        .map(|d| (d - 1) as usize)
}

#[cfg(test)]
mod alt_digit_tests {
    use super::super::*;
    use super::*;
    use iced::keyboard::Modifiers;

    fn settings_result() -> SearchResult {
        SearchResult {
            item: AppItem::scanned(
                "kite:settings".into(),
                "Kite 设置".into(),
                "kite:settings".into(),
                None,
                None,
                "builtin",
            ),
            score: 930,
            matched_by: "builtin".into(),
        }
    }

    fn state_with_settings_result(query: &str) -> State {
        State {
            data_dir: std::env::temp_dir(),
            icon_dir: std::env::temp_dir(),
            index: std::sync::Arc::new(Mutex::new(AppIndex::empty())),
            scan_options: std::sync::Arc::new(std::sync::RwLock::new(
                app::scanner::ScanOptions::default(),
            )),
            history: None,
            input_id: search_view::input_id(),
            window_id: None,
            query: query.into(),
            results: vec![settings_result()],
            selected: 0,
            hidden: false,
            ime_composing: false,
            alt_down: false,
            query_at_alt: None,
            alt_digit_consumed: false,
            index_ready: true,
            rescan_pending: false,
            files_mode: false,
            file_query_generation: 0,
            file_results: Vec::new(),
            menu: None,
            pinned: Default::default(),
            cursor: Default::default(),
            settings_open: false,
            settings_section: Section::General,
            hide_on_blur: true,
            autostart: false,
            history_recording: true,
            hotkey: "Alt+Space".into(),
            hotkey_label: "Alt+Space".into(),
            aliases: Vec::new(),
            alias_input: String::new(),
            alias_target_input: String::new(),
            alias_candidates: Vec::new(),
            alias_pick: None,
            portable_dirs: Vec::new(),
            portable_dir_input: String::new(),
            flash: None,
            hotkey_recording: false,
            update_status: None,
            update_asset: None,
            update_checking: false,
            epoch: 0,
        }
    }

    #[test]
    fn alt_digit_text_input_without_digit_key_event_launches_result() {
        for (before, after) in [("", "1"), ("k", "k1")] {
            let mut state = state_with_settings_result(before);
            let _ = update(
                &mut state,
                Message::KeyPressed(
                    Key::Named(Named::Alt),
                    Physical::Code(Code::KeyA),
                    Modifiers::empty(),
                ),
            );
            let _ = update(&mut state, Message::QueryChanged(after.into()));
            assert!(
                state.settings_open,
                "Alt+1 should open the first result from {before:?}"
            );
            assert_eq!(state.query, before, "Alt+1 must not change the query");
        }
    }

    #[test]
    fn alt_digit_key_event_then_text_input_does_not_change_opened_result_or_query() {
        let mut state = state_with_settings_result("k");
        let _ = update(
            &mut state,
            Message::KeyPressed(
                Key::Named(Named::Alt),
                Physical::Code(Code::KeyA),
                Modifiers::empty(),
            ),
        );
        let _ = update(
            &mut state,
            Message::KeyPressed(
                Key::Unidentified,
                Physical::Code(Code::Digit1),
                Modifiers::ALT,
            ),
        );
        assert!(state.settings_open);
        let _ = update(&mut state, Message::QueryChanged("k1".into()));
        assert_eq!(state.query, "k");
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn ordinary_number_input_remains_search_text() {
        let mut state = state_with_settings_result("k");
        let _ = update(&mut state, Message::QueryChanged("k1".into()));
        assert_eq!(state.query, "k1");
        assert!(!state.settings_open);
    }

    #[test]
    fn alt_digit_outside_results_does_not_launch_or_modify_query() {
        let mut state = state_with_settings_result("k");
        let _ = update(
            &mut state,
            Message::KeyPressed(
                Key::Named(Named::Alt),
                Physical::Code(Code::KeyA),
                Modifiers::empty(),
            ),
        );
        let _ = update(&mut state, Message::QueryChanged("k2".into()));
        assert_eq!(state.query, "k");
        assert!(!state.settings_open);
    }

    #[test]
    fn physical_digit_with_alt_maps_to_index() {
        let mods = Modifiers::ALT;
        let key = Key::Unidentified;
        assert_eq!(
            alt_digit_index(&key, Physical::Code(Code::Digit1), mods, false),
            Some(0)
        );
        assert_eq!(
            alt_digit_index(&key, Physical::Code(Code::Numpad9), mods, false),
            Some(8)
        );
        assert_eq!(
            alt_digit_index(&key, Physical::Code(Code::Digit0), mods, false),
            None
        );
    }

    #[test]
    fn character_digit_still_works() {
        let mods = Modifiers::ALT;
        let key: Key = Key::Character("3".into());
        assert_eq!(
            alt_digit_index(&key, Physical::Code(Code::KeyC), mods, false),
            Some(2)
        );
    }

    #[test]
    fn local_alt_down_without_modifiers_flag() {
        // SYSKEY：modifiers 可能没有 ALT，但本地跟踪到 Alt 按下
        let mods = Modifiers::empty();
        let key = Key::Unidentified;
        assert_eq!(
            alt_digit_index(&key, Physical::Code(Code::Digit2), mods, true),
            Some(1)
        );
    }

    #[test]
    fn without_alt_is_none() {
        let mods = Modifiers::empty();
        let key: Key = Key::Character("1".into());
        assert_eq!(
            alt_digit_index(&key, Physical::Code(Code::Digit1), mods, false),
            None
        );
    }
}
