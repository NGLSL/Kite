#![cfg(test)]

use super::super::test_support::test_state;
use super::super::NavigationMode;
use super::*;
use crate::model::{AppItem, SearchResult};
use crate::plugin::{Activation, PanelBlock, PanelData, ResponseMode};
use crate::ui::theme::ThemeMode;

#[test]
fn test_view_empty_query_grid_dashboard() {
    let mut state = test_state("");
    state.results = (0..16)
        .map(|i| {
            SearchResult::scored(
                AppItem::scanned(
                    format!("app:{i}"),
                    format!("App {i}"),
                    format!("C:\\app_{i}.exe"),
                    None,
                    None,
                    "test",
                ),
                100 - i as i32,
                "test",
            )
        })
        .collect();

    // 深色模式渲染无 panic
    state.theme_mode = ThemeMode::Dark;
    {
        let _elem = view(&state);
    }

    // 浅色模式渲染无 panic
    state.theme_mode = ThemeMode::Light;
    {
        let _elem = view(&state);
    }
}

#[test]
fn test_view_active_search_two_column() {
    let mut state = test_state("test");
    state.results = (0..8)
        .map(|i| {
            SearchResult::scored(
                AppItem::scanned(
                    format!("app:{i}"),
                    format!("App {i}"),
                    format!("C:\\app_{i}.exe"),
                    None,
                    None,
                    "test",
                ),
                100 - i as i32,
                "test",
            )
        })
        .collect();

    state.theme_mode = ThemeMode::Dark;
    {
        let _elem = view(&state);
    }

    state.theme_mode = ThemeMode::Light;
    {
        let _elem = view(&state);
    }

    state.files_mode = true;
    state.file_filter = crate::system::everything::FileFilter::Images;
    state.theme_mode = ThemeMode::Dark;
    {
        let _elem = view(&state);
    }
    state.theme_mode = ThemeMode::Light;
    {
        let _elem = view(&state);
    }
}

#[test]
fn test_view_provider_mode_instant_card() {
    let mut state = test_state("=");
    state.provider_mode = Some(Activation {
        plugin_id: "com.kite.calculator".into(),
        provider_id: "calculate".into(),
        effective_query: "1+1".into(),
        raw_query: "=1+1".into(),
        response_mode: ResponseMode::Panel,
    });
    state.plugin_panel = Some(PanelData {
        blocks: vec![
            PanelBlock::Value {
                label: Some("计算结果".into()),
                value: "2".into(),
                selectable: Some(true),
            },
            PanelBlock::KeyValue {
                items: vec![crate::plugin::panel::KeyValueItem {
                    key: "HEX".into(),
                    value: "0x2".into(),
                }],
            },
        ],
        actions: vec![],
    });

    state.theme_mode = ThemeMode::Dark;
    {
        let _elem = view(&state);
    }

    state.theme_mode = ThemeMode::Light;
    {
        let _elem = view(&state);
    }
}

#[test]
fn test_grid_2d_keyboard_navigation() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("");
    state.results = (0..20)
        .map(|i| {
            SearchResult::scored(
                AppItem::scanned(
                    format!("app:{i}"),
                    format!("App {i}"),
                    format!("C:\\app_{i}.exe"),
                    None,
                    None,
                    "test",
                ),
                100 - i as i32,
                "test",
            )
        })
        .collect();

    // 输入模式：左右键交给输入框，不改变结果选择。
    state.selected = 0;
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowRight),
        Physical::Code(Code::ArrowRight),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 0);
    assert_eq!(state.navigation_mode, NavigationMode::Input);

    // 第一次 ArrowDown 只进入结果导航并选中第一项。
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowDown),
        Physical::Code(Code::ArrowDown),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 0);
    assert_eq!(state.navigation_mode, NavigationMode::Results);

    // 空 Query：结果导航模式下 ArrowDown +8
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowDown),
        Physical::Code(Code::ArrowDown),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 8);

    // 空 Query：ArrowRight +1
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowRight),
        Physical::Code(Code::ArrowRight),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 9);

    // 空 Query：ArrowUp -8
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowUp),
        Physical::Code(Code::ArrowUp),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 1);

    // 非空 Query：ArrowDown +1, ArrowRight +4
    state.query = "app".into();
    state.selected = 0;
    state.navigation_mode = NavigationMode::Input;
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowDown),
        Physical::Code(Code::ArrowDown),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 0);

    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowDown),
        Physical::Code(Code::ArrowDown),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 1);

    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowRight),
        Physical::Code(Code::ArrowRight),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 5);

    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowLeft),
        Physical::Code(Code::ArrowLeft),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 1);

    // 结果导航模式下 Esc 返回输入框，不隐藏窗口。
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::Escape),
        Physical::Code(Code::Escape),
        Modifiers::empty(),
    );
    assert_eq!(state.navigation_mode, NavigationMode::Input);
    assert!(!state.hidden);
}

#[test]
fn test_single_column_results_support_horizontal_navigation() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("xj");
    state.results = (0..3)
        .map(|i| {
            SearchResult::scored(
                AppItem::scanned(
                    format!("app:{i}"),
                    format!("App {i}"),
                    format!("C:\\app_{i}.exe"),
                    None,
                    None,
                    "test",
                ),
                100 - i as i32,
                "test",
            )
        })
        .collect();
    state.navigation_mode = NavigationMode::Results;
    state.selected = 0;

    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowRight),
        Physical::Code(Code::ArrowRight),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 1, "单列结果按右键应当移动到下一项");

    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowLeft),
        Physical::Code(Code::ArrowLeft),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 0, "单列结果按左键应当回到上一项");
}

#[test]
fn test_provider_mode_backspace_exits() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("");
    state.provider_mode = Some(Activation {
        plugin_id: "com.kite.calculator".into(),
        provider_id: "calculate".into(),
        effective_query: "".into(),
        raw_query: "=".into(),
        response_mode: ResponseMode::Panel,
    });

    // 在 query 为空时按 Backspace 退出 provider mode
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::Backspace),
        Physical::Code(Code::Backspace),
        Modifiers::empty(),
    );
    assert!(state.provider_mode.is_none());
}

#[test]
fn test_backspace_exits_files_mode() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("");
    state.files_mode = true;
    state.file_filter = crate::system::everything::FileFilter::Images;

    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::Backspace),
        Physical::Code(Code::Backspace),
        Modifiers::empty(),
    );
    assert!(!state.files_mode);
    assert_eq!(
        state.file_filter,
        crate::system::everything::FileFilter::All
    );
}

#[test]
fn test_empty_grid_with_few_items_and_pinned() {
    let mut state = test_state("");
    // 3 recent items
    state.results = (0..3)
        .map(|i| {
            SearchResult::scored(
                AppItem::scanned(
                    format!("app:{i}"),
                    format!("App {i}"),
                    format!("C:\\app_{i}.exe"),
                    None,
                    None,
                    "test",
                ),
                100 - i as i32,
                "test",
            )
        })
        .collect();
    state.grid_recent_count = 3;

    // 2 pinned items appended
    for i in 3..5 {
        state.results.push(SearchResult::scored(
            AppItem::scanned(
                format!("pinned:{i}"),
                format!("Pinned {i}"),
                format!("C:\\pinned_{i}.exe"),
                None,
                None,
                "test",
            ),
            10,
            "pinned",
        ));
    }

    // 必须安全渲染，不依赖 16 项假定
    let _elem = view(&state);
}

#[test]
fn test_ctrl_enter_in_provider_mode() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("=");
    state.provider_mode = Some(Activation {
        plugin_id: "com.kite.calculator".into(),
        provider_id: "calculate".into(),
        effective_query: "1+1".into(),
        raw_query: "=1+1".into(),
        response_mode: ResponseMode::Panel,
    });
    state.plugin_panel = Some(PanelData {
        blocks: vec![PanelBlock::Value {
            label: Some("结果".into()),
            value: "2".into(),
            selectable: Some(true),
        }],
        actions: vec![],
    });

    // 按 Ctrl+Enter 触发复制并不关闭窗口
    let mut mods = Modifiers::empty();
    mods.set(Modifiers::CTRL, true);
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::Enter),
        Physical::Code(Code::Enter),
        mods,
    );
    assert!(!state.hidden, "Ctrl+Enter 复制并继续不得关闭窗口");
    assert!(state.flash.is_some(), "应产生已复制提示");
}

#[test]
fn test_truncate_display_label() {
    assert_eq!(truncate_display_label("ChatGPT", 11), "ChatGPT");
    assert_eq!(truncate_display_label("XTerminal", 11), "XTerminal");
    assert_eq!(truncate_display_label("WeGame", 11), "WeGame");
    assert_eq!(truncate_display_label("微信", 11), "微信");
    assert_eq!(truncate_display_label("QQ", 11), "QQ");
    assert_eq!(truncate_display_label("Antigravity", 11), "Antigravity");
    assert_eq!(
        truncate_display_label("Visual Studio Code", 11),
        "Visual Stu…"
    );
}

#[test]
fn test_two_column_navigation_cross_chunks() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("test");
    state.results = (0..16)
        .map(|i| {
            SearchResult::scored(
                AppItem::scanned(
                    format!("app:{i}"),
                    format!("App {i}"),
                    format!("C:\\app_{i}.exe"),
                    None,
                    None,
                    "test",
                ),
                100 - i as i32,
                "test",
            )
        })
        .collect();

    // 初始在 0（左列第 1 行）
    state.selected = 0;
    state.navigation_mode = NavigationMode::Results;

    // 向下 3 次到达 3（左列第 4 行，即块底部）
    for _ in 0..3 {
        let _ = on_key(
            &mut state,
            iced::keyboard::Key::Named(Named::ArrowDown),
            Physical::Code(Code::ArrowDown),
            Modifiers::empty(),
        );
    }
    assert_eq!(state.selected, 3);

    // 再次向下：应当跨块跳到 8（下一块左列第 1 行），而非跳到 4（右列第 1 行）
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowDown),
        Physical::Code(Code::ArrowDown),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 8, "在左列底部按向下应当进入下一行的左列(8)");

    // 向上：应当跨块回退到 3（上一块左列第 4 行）
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowUp),
        Physical::Code(Code::ArrowUp),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 3, "在左列跨块向上应当回到上一块左列(3)");

    // 向右：进入同一行的右列 (3 + 4 = 7)
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowRight),
        Physical::Code(Code::ArrowRight),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 7, "向右跨列移动");

    // 向左：回退到同一行的左列 (7 - 4 = 3)
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::ArrowLeft),
        Physical::Code(Code::ArrowLeft),
        Modifiers::empty(),
    );
    assert_eq!(state.selected, 3, "向左跨列移动");
}

#[test]
fn test_ctrl_enter_without_result_shows_flash() {
    use crate::ui::actions::on_key;
    use iced::keyboard::key::{Code, Named, Physical};
    use iced::keyboard::Modifiers;

    let mut state = test_state("=");
    state.provider_mode = Some(Activation {
        plugin_id: "com.kite.calculator".into(),
        provider_id: "calculate".into(),
        effective_query: "".into(),
        raw_query: "=".into(),
        response_mode: ResponseMode::Panel,
    });
    state.plugin_panel = None;

    let mut mods = Modifiers::empty();
    mods.set(Modifiers::CTRL, true);
    let _ = on_key(
        &mut state,
        iced::keyboard::Key::Named(Named::Enter),
        Physical::Code(Code::Enter),
        mods,
    );
    assert_eq!(state.flash.as_deref(), Some("暂无可复制的计算结果"));

    // 带有 Toast 浮层时 view 正常渲染
    let _elem = view(&state);
}
