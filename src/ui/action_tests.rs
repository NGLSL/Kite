//! 结果操作面板的消息交互回归。
//!
//! 这些测试从 `State/update` 入口观察用户可见状态：面板捕获的目标、Alias
//! 编辑器、Query 快照以及隐藏/恢复后的结果列表。它们不依赖 Iced widget
//! 的渲染树，也不 mock 历史库。

use super::*;
use crate::model::{AppItem, SearchResult};
use crate::storage::HistoryDb;
use iced::keyboard::key::{Code, Named};

use super::actions::{load_aliases, on_key};
use super::interaction::update;
use super::test_support::test_state;

fn temp_db(tag: &str) -> HistoryDb {
    let path = std::env::temp_dir().join(format!(
        "kite-ui-action-{tag}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    HistoryDb::open(&path).expect("test history db")
}

fn app(id: &str) -> AppItem {
    let mut item = AppItem::scanned(
        id.to_owned(),
        format!("App {id}"),
        format!(r"C:\Apps\{id}.exe"),
        None,
        None,
        "manual",
    );
    item.attach_search_fields();
    item
}

fn state_with_app(query: &str, item: AppItem) -> State {
    let mut state = test_state(query);
    state.hidden = false;
    state.history = Some(temp_db("state"));
    state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .apps = vec![item.clone()];
    state.results = vec![SearchResult::scored(item, 1000, "exact")];
    state
}

fn open_alias_editor(state: &mut State) -> AppItem {
    let item = state.results[0].item.clone();
    let _ = update(state, Message::ContextMenu(0));
    let captured = state
        .menu
        .as_ref()
        .map(|(item, _, _)| item.clone())
        .expect("context menu captures the result");
    let _ = update(state, Message::MenuAction(captured, MenuAction::SetAlias));
    assert_eq!(
        state.action_alias_item.as_ref().map(|value| &value.id),
        Some(&item.id)
    );
    item
}

#[test]
fn disappeared_captured_target_refuses_mutating_action() {
    let item = app("gone");
    let mut state = state_with_app("gone", item);
    let _ = update(&mut state, Message::ContextMenu(0));
    let captured = state
        .menu
        .as_ref()
        .map(|(item, _, _)| item.clone())
        .expect("context menu captures the result");

    state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .apps
        .clear();
    let _ = update(
        &mut state,
        Message::MenuAction(captured, MenuAction::HideEntry),
    );

    assert!(
        state
            .history
            .as_ref()
            .expect("history")
            .hidden_ids()
            .expect("hidden ids")
            .is_empty(),
        "过期目标不得写入隐藏记录"
    );
    assert!(state.menu.is_none(), "拒绝过期动作后应关闭面板");
    assert!(
        state
            .flash
            .as_deref()
            .is_some_and(|message| message.contains("目标")),
        "拒绝原因应反馈给用户"
    );
}

#[test]
fn direct_alias_save_binds_the_captured_target_and_closes_editor() {
    let item = app("direct-alias");
    let mut state = state_with_app("VS", item.clone());
    let _ = open_alias_editor(&mut state);
    let _ = update(
        &mut state,
        Message::ActionAliasInputChanged("Visual Studio".into()),
    );
    let _ = update(&mut state, Message::ActionAliasSave);

    let aliases = state
        .history
        .as_ref()
        .expect("history")
        .alias_matches("visual studio");
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].target_id.as_deref(), Some(item.id.as_str()));
    assert!(state.action_alias_item.is_none(), "保存后应退出 Alias 编辑");
    assert!(state.action_alias_conflict.is_none());
    assert!(state.menu.is_none(), "保存后应关闭结果面板");
}

#[test]
fn alias_conflict_can_be_cancelled_without_changing_original_mapping() {
    let target = app("alias-new");
    let old_target = app("alias-old");
    let mut state = state_with_app("VS", target.clone());
    state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .apps
        .push(old_target.clone());
    state
        .history
        .as_mut()
        .expect("history")
        .set_alias("vs", Some(&old_target.id), &old_target.display_name)
        .expect("existing alias");
    load_aliases(&mut state);

    let captured = open_alias_editor(&mut state);
    let _ = update(&mut state, Message::ActionAliasSave);
    assert!(state.action_alias_conflict.is_some());
    assert_eq!(
        state.history.as_ref().expect("history").alias_matches("vs")[0]
            .target_id
            .as_deref(),
        Some(old_target.id.as_str())
    );

    let _ = update(&mut state, Message::ActionAliasCancel);
    assert!(state.action_alias_item.is_none());
    assert!(state.action_alias_conflict.is_none());
    assert!(state.menu.is_some(), "取消编辑应回到当前结果面板");
    assert_eq!(
        state.history.as_ref().expect("history").alias_matches("vs")[0]
            .target_id
            .as_deref(),
        Some(old_target.id.as_str()),
        "取消冲突不得覆盖原 Alias"
    );
    let _ = captured;
}

#[test]
fn alias_conflict_replace_moves_mapping_to_the_captured_target() {
    let target = app("alias-new");
    let old_target = app("alias-old");
    let mut state = state_with_app("VS", target.clone());
    state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .apps
        .push(old_target.clone());
    state
        .history
        .as_mut()
        .expect("history")
        .set_alias("vs", Some(&old_target.id), &old_target.display_name)
        .expect("existing alias");
    load_aliases(&mut state);

    open_alias_editor(&mut state);
    let _ = update(&mut state, Message::ActionAliasSave);
    assert!(state.action_alias_conflict.is_some());
    let _ = update(&mut state, Message::ActionAliasReplace);

    let aliases = state.history.as_ref().expect("history").alias_matches("vs");
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].target_id.as_deref(), Some(target.id.as_str()));
    assert!(state.action_alias_item.is_none());
    assert!(state.action_alias_conflict.is_none());
    assert!(state.menu.is_none());
}

#[test]
fn alias_save_error_keeps_editor_input_and_target() {
    let item = app("alias-error");
    let mut state = state_with_app("VS", item.clone());
    state.history = None;
    open_alias_editor(&mut state);
    let _ = update(
        &mut state,
        Message::ActionAliasInputChanged("typed while unavailable".into()),
    );
    let _ = update(&mut state, Message::ActionAliasSave);

    assert_eq!(state.action_alias_input, "typed while unavailable");
    assert_eq!(
        state
            .action_alias_item
            .as_ref()
            .map(|value| value.id.as_str()),
        Some(item.id.as_str())
    );
    assert!(state.menu.is_some(), "失败后仍应保留编辑上下文");
    assert!(state
        .flash
        .as_deref()
        .is_some_and(|message| message.contains("无法保存 Alias")));
}

#[test]
fn ime_enter_does_not_save_alias_during_composition() {
    let item = app("ime-alias");
    let mut state = state_with_app("VS", item.clone());
    open_alias_editor(&mut state);
    state.action_alias_input = "中文候选".into();
    state.ime_composing = true;

    let _ = on_key(
        &mut state,
        Key::Named(Named::Enter),
        Physical::Code(Code::Enter),
        Modifiers::empty(),
    );

    assert!(state
        .history
        .as_ref()
        .expect("history")
        .alias_matches("中文候选")
        .is_empty());
    assert_eq!(state.action_alias_input, "中文候选");
    assert_eq!(
        state
            .action_alias_item
            .as_ref()
            .map(|value| value.id.as_str()),
        Some(item.id.as_str())
    );
}

#[test]
fn escape_exits_alias_editor_then_panel_then_search() {
    let item = app("esc-alias");
    let mut state = state_with_app("keep query", item);
    open_alias_editor(&mut state);
    state.action_alias_input = "draft".into();

    let _ = on_key(
        &mut state,
        Key::Named(Named::Escape),
        Physical::Code(Code::Escape),
        Modifiers::empty(),
    );
    assert!(state.action_alias_item.is_none());
    assert!(state.menu.is_some(), "第一层 Esc 只应退出 Alias 编辑");

    let _ = on_key(
        &mut state,
        Key::Named(Named::Escape),
        Physical::Code(Code::Escape),
        Modifiers::empty(),
    );
    assert!(state.menu.is_none(), "第二层 Esc 应关闭结果面板");
    assert_eq!(state.query, "keep query", "关闭面板应保留 Query");

    let _ = on_key(
        &mut state,
        Key::Named(Named::Escape),
        Physical::Code(Code::Escape),
        Modifiers::empty(),
    );
    assert!(state.hidden, "第三层 Esc 才隐藏搜索窗口");
}

#[test]
fn hide_and_restore_updates_empty_query_results_without_consuming_visible_slot() {
    let hidden = app("hidden-empty");
    let visible = app("visible-empty");
    let mut state = state_with_app("", hidden.clone());
    state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .apps
        .push(visible.clone());
    state
        .history
        .as_mut()
        .expect("history")
        .record_launch(&hidden.id, "open", 10)
        .expect("recent item");
    state.results = vec![
        SearchResult::scored(hidden.clone(), 1000, "recent"),
        SearchResult::scored(visible.clone(), 900, "default"),
    ];

    let _ = update(&mut state, Message::ContextMenu(0));
    let captured = state
        .menu
        .as_ref()
        .map(|(item, _, _)| item.clone())
        .expect("context menu captures hidden candidate");
    let _ = update(
        &mut state,
        Message::MenuAction(captured, MenuAction::HideEntry),
    );

    assert!(state.hidden_ids.contains(&hidden.id));
    assert!(!state
        .results
        .iter()
        .any(|result| result.item.id == hidden.id));
    assert!(
        state
            .results
            .iter()
            .any(|result| result.item.id == visible.id),
        "隐藏项被移除后应由可见应用填充空查询列表"
    );

    let _ = update(&mut state, Message::RestoreHidden(hidden.id.clone()));
    assert!(!state.hidden_ids.contains(&hidden.id));
    assert!(
        state
            .results
            .iter()
            .any(|result| result.item.id == hidden.id),
        "恢复后空查询应重新展示原入口"
    );
}
