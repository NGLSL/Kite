//! Iced 消息到界面状态的更新：按域分发，工具/搜索/窗口/设置/插件各自处理。

use super::*;

/// 统一 update 入口。行为与拆分前一致；各域实现见 `update_*` 与 `tools`。
pub(super) fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Tool(..)
        | Message::ToolWindowClosed(..)
        | Message::PluginOpenTool(..)
        | Message::PluginConfirmTool
        | Message::PluginCancelToolConfirm => update_plugins::handle(state, message),

        Message::QueryChanged(..)
        | Message::ClearQuery
        | Message::HoverSelect(..)
        | Message::LaunchIndex(..)
        | Message::ToggleFiles
        | Message::FileFilterChanged(..)
        | Message::FileSearchReady(..)
        | Message::DirectPathReady(..)
        | Message::AppSearchReady(..)
        | Message::CursorMoved(..)
        | Message::MousePressed(..)
        | Message::ContextMenu(..)
        | Message::MenuAction(..)
        | Message::Rescan => update_search::handle(state, message),

        Message::OpenSettings
        | Message::CloseSettings
        | Message::SettingsSection(..)
        | Message::SetAutostart(..)
        | Message::SetHideOnBlur(..)
        | Message::SetHistoryRecording(..)
        | Message::SetThemeMode(..)
        | Message::SetQueryLog(..)
        | Message::SetSearchEngine(..)
        | Message::SearchEngineCustomChanged(..)
        | Message::SetWebSearchHotkey(..)
        | Message::ClearHistory
        | Message::ApplyHotkey(..)
        | Message::AliasInputChanged(..)
        | Message::AliasTargetChanged(..)
        | Message::AliasPick(..)
        | Message::AliasAdd
        | Message::AliasRemove(..)
        | Message::PortableDirInputChanged(..)
        | Message::AddPortableDir
        | Message::RemovePortableDir(..)
        | Message::StartHotkeyRecord
        | Message::HotkeyRecordCaptured(..) => update_settings::handle(state, message),

        Message::PluginQueryReady(..)
        | Message::PluginHostCall(..)
        | Message::PluginSetEnabled(..)
        | Message::PluginReload(..)
        | Message::PluginOpenDir(..)
        | Message::PluginOpenLog(..)
        | Message::PluginUninstall(..)
        | Message::PluginImportPathChanged(..)
        | Message::PluginImportFromPath
        | Message::PluginInstallOfficial
        | Message::PluginOpenPluginsDir
        | Message::PluginRescanPlugins
        | Message::PluginTryExample(..)
        | Message::PluginToggleDocs(..) => update_plugins::handle(state, message),

        // 窗口生命周期 / 热键 / 索引 / 更新
        _ => update_window::handle(state, message),
    }
}

/// 采用后台 Full 挂起快照；调用方随后刷新当前查询。
pub(super) fn apply_pending_full(state: &mut State) -> bool {
    let Some(full) = super::backend::take_pending_full() else {
        return false;
    };
    let count = full.apps.len();
    let applied = state
        .index
        .lock()
        .map(|mut current| {
            *current = full;
            true
        })
        .unwrap_or(false);
    if !applied {
        return false;
    }
    plog(&format!("applied pending full snapshot n={count}"));
    // 旧含 source 的 id 迁移到 stable id，保留 Pin/历史/Alias
    if let Some(db) = &mut state.history {
        let items: Vec<(String, String, Option<String>)> = {
            let g = state.index.lock().unwrap_or_else(|e| e.into_inner());
            g.apps
                .iter()
                .map(|a| (a.id.clone(), a.target.clone(), a.args.clone()))
                .collect()
        };
        let moved = db.migrate_legacy_ids_for_items(&items);
        if moved > 0 {
            plog(&format!("identity remap moved={moved}"));
        }
        state.pinned = db.pinned_ids().into_iter().collect();
    }
    state.invalidate_prefs_cache();
    state.index_generation = state.index_generation.wrapping_add(1);
    state.base_hit_cache.clear();
    true
}

pub(super) fn capture_menu_item(results: &[SearchResult], index: usize) -> Option<AppItem> {
    results.get(index).map(|result| result.item.clone())
}

/// 导入结果落地：Registry upsert + Host reload + 刷新搜索。
pub(super) fn apply_plugin_import_results(
    state: &mut State,
    results: Vec<Result<plugin::ImportOutcome, String>>,
) -> Task<Message> {
    let mut ok = 0usize;
    let mut replaced = 0usize;
    let mut errs: Vec<String> = Vec::new();
    for r in results {
        match r {
            Ok(out) => {
                super::with_plugin_host(state, |host| host.reload(&out.plugin_id));
                let reg_result = super::with_plugin_registry(state, |reg| {
                    plugin::apply_import_to_registry(reg, &out.dest)
                });
                match reg_result {
                    Ok(id) => {
                        state.qlog(|| {
                            format!(
                                "plugin import ok id={id} dest={} replaced={}",
                                out.dest.display(),
                                out.replaced
                            )
                        });
                        ok += 1;
                        if out.replaced {
                            replaced += 1;
                        }
                    }
                    Err(e) => {
                        state.qlog(|| format!("plugin import registry err={e}"));
                        errs.push(e);
                    }
                }
            }
            Err(e) => {
                state.qlog(|| format!("plugin import err={e}"));
                errs.push(e);
            }
        }
    }
    state.refresh_results();
    if ok == 0 {
        let msg = errs.first().cloned().unwrap_or_else(|| "导入失败".into());
        actions::flash(state, &msg)
    } else if errs.is_empty() {
        actions::flash(
            state,
            &format!(
                "已导入 {ok} 个插件{}",
                if replaced > 0 { "（含覆盖）" } else { "" }
            ),
        )
    } else {
        actions::flash(state, &format!("导入 {ok} 个成功，{} 个失败", errs.len()))
    }
}

pub(super) fn persist_portable_dirs(state: &mut State) {
    if let Some(db) = &mut state.history {
        if let Ok(saved) = db.set_portable_dirs(&state.portable_dirs) {
            state.portable_dirs = saved;
        }
    }
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

pub(super) fn request_rescan(state: &State) {
    if let Some(tx) = event_tx() {
        backend::request_build(
            state.index.clone(),
            state.icon_dir.clone(),
            state.scan_options.clone(),
            tx,
        );
    }
}

#[cfg(test)]
mod plugin_ui_tests {
    use super::super::test_support::test_state;
    use super::*;
    use crate::model::{ResultAction, ResultSource, SearchResult};
    use crate::plugin::activation::{Activation, ResponseMode};
    use crate::plugin::manifest::{
        CommandAction, Compatibility, Contributions, PluginCommand, PluginIdentity, PluginProvider,
        RuntimeSpec,
    };
    use crate::plugin::registry::{RegisteredPlugin, RuntimePhase};
    use crate::plugin::Trigger;
    use crate::ui::tools::{ToolKind, ToolOp};
    use iced::keyboard::key::Code;

    fn calculator_plugin() -> RegisteredPlugin {
        RegisteredPlugin {
            manifest: crate::plugin::PluginManifest {
                schema_version: 1,
                plugin: PluginIdentity {
                    id: "com.kite.calculator".into(),
                    name: "计算器".into(),
                    version: "0.1.0".into(),
                    description: "在搜索框输入 = 直接计算".into(),
                    usage: "在搜索框输入 = 加上算式。\n示例：=1+1\nEnter 复制结果。".into(),
                    author: String::new(),
                },
                compatibility: Compatibility {
                    plugin_api: 1,
                    minimum_kite_version: None,
                },
                runtime: RuntimeSpec {
                    command: "calculator.exe".into(),
                    args: vec![],
                    startup_timeout_ms: Some(5000),
                    idle_timeout_ms: Some(60_000),
                },
                contributes: Contributions {
                    examples: vec!["=1+1".into(), "=1+1*(2+1)".into()],
                    commands: vec![PluginCommand {
                        id: "open".into(),
                        title: "计算器".into(),
                        keywords: vec!["calc".into()],
                        action: CommandAction::EnterProvider {
                            provider: "calculate".into(),
                        },
                    }],
                    providers: vec![PluginProvider {
                        id: "calculate".into(),
                        response_mode: "panel".into(),
                        triggers: vec![Trigger::Prefix { value: "=".into() }],
                    }],
                },
            },
            root: std::path::PathBuf::from("plugins/com.kite.calculator"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        }
    }

    fn devtools_plugin() -> RegisteredPlugin {
        RegisteredPlugin {
            manifest: crate::plugin::PluginManifest {
                schema_version: 1,
                plugin: PluginIdentity {
                    id: "com.kite.devtools".into(),
                    name: "开发者工具".into(),
                    version: "0.1.0".into(),
                    description: "JSON / Hash / Base64".into(),
                    usage: "json / hash / base64".into(),
                    author: String::new(),
                },
                compatibility: Compatibility {
                    plugin_api: 1,
                    minimum_kite_version: None,
                },
                runtime: RuntimeSpec {
                    command: "devtools.exe".into(),
                    args: vec![],
                    startup_timeout_ms: Some(5000),
                    idle_timeout_ms: Some(60_000),
                },
                contributes: Contributions {
                    examples: vec!["json".into()],
                    commands: vec![],
                    providers: vec![
                        PluginProvider {
                            id: "json".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Keyword {
                                value: "json".into(),
                            }],
                        },
                        PluginProvider {
                            id: "hash".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Keyword {
                                value: "hash".into(),
                            }],
                        },
                        PluginProvider {
                            id: "base64".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Keyword {
                                value: "base64".into(),
                            }],
                        },
                    ],
                },
            },
            root: std::path::PathBuf::from("plugins/com.kite.devtools"),
            enabled: true,
            phase: RuntimePhase::Dormant,
            last_error: None,
        }
    }

    #[test]
    fn provider_mode_enters_on_trigger_and_esc_exits() {
        let mut state = test_state("=1+2");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(calculator_plugin());
        }
        let act = {
            let reg = state.plugin_registry.lock().unwrap();
            plugin::route_query(&state.query, &reg).expect("activation")
        };
        assert_eq!(act.provider_id, "calculate");
        state.provider_mode = Some(act);
        state.plugin_panel = Some(
            plugin::parse_panel(&serde_json::json!({
                "blocks": [{ "type": "value", "value": "3" }],
                "actions": [{
                    "id": "copy",
                    "label": "复制结果",
                    "default": true,
                    "action": { "type": "copy_text", "text": "3" }
                }]
            }))
            .unwrap(),
        );

        let _ = actions::on_key(
            &mut state,
            Key::Named(Named::Escape),
            Physical::Code(Code::Escape),
            Modifiers::empty(),
        );
        assert!(state.provider_mode.is_none(), "Esc 退出 Provider Mode");
        assert!(state.plugin_panel.is_none());
        assert!(state.query.is_empty());
    }

    #[test]
    fn ordinary_query_with_plugins_stays_core() {
        let mut state = test_state("chrome");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(calculator_plugin());
        }
        state.refresh_results();
        assert!(
            state.provider_mode.is_none(),
            "普通查询不得进入 Provider Mode"
        );
    }

    #[test]
    fn install_official_samples_registers_plugins() {
        let mut state = test_state("");
        crate::system::resources::init(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let _ = update(&mut state, Message::PluginInstallOfficial);
        let reg = state.plugin_registry.lock().unwrap();
        assert!(
            reg.get("com.kite.calculator").is_some(),
            "官方 Calculator 应已注册"
        );
        assert!(reg.get("com.kite.window-switcher").is_some());
        assert!(reg.get("com.kite.devtools").is_some());
    }

    #[test]
    fn import_from_path_rejects_empty_and_registers_folder() {
        let mut state = test_state("");
        let _ = update(&mut state, Message::PluginImportFromPath);
        assert!(state.flash.as_deref().is_some_and(|f| f.contains("路径")));

        let tmp = std::env::temp_dir().join(format!("kite-ui-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("com.kite.frompath");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("x.exe"), b"MZ").unwrap();
        std::fs::write(
            src.join("plugin.json"),
            serde_json::json!({
                "schema_version": 1,
                "plugin": {
                    "id": "com.kite.frompath",
                    "name": "FromPath",
                    "version": "0.1.0",
                    "description": "d",
                    "usage": "u",
                    "author": "a"
                },
                "compatibility": { "plugin_api": 1 },
                "runtime": { "command": "x.exe" },
                "contributes": {
                    "examples": ["fp"],
                    "commands": [],
                    "providers": [{
                        "id": "main",
                        "response_mode": "list",
                        "triggers": [{ "type": "keyword", "value": "fp" }]
                    }]
                }
            })
            .to_string(),
        )
        .unwrap();
        state.plugin_import_path = src.to_string_lossy().to_string();
        let _ = update(&mut state, Message::PluginImportFromPath);
        let reg = state.plugin_registry.lock().unwrap();
        assert!(reg.get("com.kite.frompath").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn search_json_keyword_opens_independent_panel_not_provider() {
        use actions::launch_selected;
        let mut state = test_state("json");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(devtools_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged("json".into()));
        assert!(
            !state.any_tool_open() && state.tools.json.window_id.is_none(),
            "搜索 json 不得直接打开工具窗，须二次确认"
        );
        assert!(state.provider_mode.is_none(), "不得进入 Provider");
        assert!(state.pending_tool_confirm.is_none());
        state.apply_app_search_ready(
            state.app_query_generation,
            state.query.clone(),
            vec![],
            0,
            state.index_generation,
        );
        assert!(state
            .results
            .iter()
            .any(|r| r.item.id == "plugin-trigger:com.kite.devtools:json"));
        state.selected = state
            .results
            .iter()
            .position(|r| r.item.id == "plugin-trigger:com.kite.devtools:json")
            .unwrap();
        let _ = launch_selected(&mut state);
        assert!(
            state.any_tool_open() && state.tools.json.window_id.is_some(),
            "确认后才打开独立工具窗"
        );
        assert!(state.pending_tool_confirm.is_none());

        let mut state = test_state("json");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(devtools_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged("json".into()));
        assert!(state.pending_tool_confirm.is_none());
        let _ = actions::on_key(
            &mut state,
            Key::Named(Named::Escape),
            Physical::Code(Code::Escape),
            Modifiers::empty(),
        );
        assert!(state.pending_tool_confirm.is_none());
        assert!(!state.any_tool_open(), "取消不得打开工具窗");

        let mut state = test_state("");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(devtools_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged(r#"json {"a":1}"#.into()));
        assert!(!state.any_tool_open(), "带 payload 也不得直接开窗");
        assert!(state.pending_tool_confirm.is_none());
        state.apply_app_search_ready(
            state.app_query_generation,
            state.query.clone(),
            vec![],
            0,
            state.index_generation,
        );
        state.selected = state
            .results
            .iter()
            .position(|r| r.item.id == "plugin-trigger:com.kite.devtools:json")
            .unwrap();
        let _ = launch_selected(&mut state);
        assert!(state.any_tool_open());
        assert!(state.tools.json.result.contains("\"a\": 1"));
    }

    #[test]
    fn json_tool_opens_as_independent_panel_not_settings_subpage() {
        let mut state = test_state("");
        state.settings_open = true;
        state.settings_section = Section::Plugins;
        let _ = update(&mut state, Message::PluginOpenTool(ToolKind::Json));
        assert!(
            !state.any_tool_open() && state.tools.json.window_id.is_none(),
            "设置页打开面板也须二次确认"
        );
        assert!(state
            .pending_tool_confirm
            .as_ref()
            .is_some_and(|p| p.from_settings));
        assert!(state.settings_open, "确认态不改主窗设置页");

        let _ = update(&mut state, Message::PluginConfirmTool);
        assert!(state.any_tool_open());
        assert!(state.tools.json.window_id.is_some(), "工具应是独立 OS 窗口");
        assert!(
            state.settings_open,
            "打开 JSON 工具不得改动主启动器窗口（设置页保持原样）"
        );
        assert_eq!(state.settings_section, Section::Plugins);
        assert!(state.plugin_docs_open.is_none());

        state.tools.json.editor =
            iced::widget::text_editor::Content::with_text(r#"{"a":1,"b":[true,null]}"#);
        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Json, ToolOp::Transform { minify: false }),
        );
        assert!(state.tools.json.result.contains("\"a\": 1"));

        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Json, ToolOp::Transform { minify: true }),
        );
        assert!(!state.tools.json.result.contains('\n'));

        let _ = update(&mut state, Message::Tool(ToolKind::Json, ToolOp::Close));
        assert!(!state.any_tool_open());
        assert!(state.tools.json.window_id.is_none());
        assert!(state.settings_open, "关闭工具窗后主窗设置页仍应保持打开");
    }

    #[test]
    fn settings_json_tool_confirm_can_cancel_without_opening() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenTool(ToolKind::Json));
        assert!(state.pending_tool_confirm.is_some());
        let _ = update(&mut state, Message::PluginCancelToolConfirm);
        assert!(state.pending_tool_confirm.is_none());
        assert!(!state.any_tool_open());
        assert!(state.settings_open);
    }

    #[test]
    fn blur_keeps_json_tool_panel_open() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenTool(ToolKind::Json));
        let _ = update(&mut state, Message::PluginConfirmTool);
        let _ = update(&mut state, Message::WindowBlur);
        assert!(
            state.any_tool_open() && state.tools.json.window_id.is_some(),
            "工具窗独立存在，主窗失焦不得关闭它"
        );
        assert!(!state.hidden);
    }

    #[test]
    fn plugin_docs_open_settings_but_json_tool_is_separate() {
        let mut state = test_state("");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(calculator_plugin());
        }
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenTool(ToolKind::Json));
        let _ = update(&mut state, Message::PluginConfirmTool);
        assert!(state.any_tool_open());
        let _ = update(
            &mut state,
            Message::PluginToggleDocs("com.kite.calculator".into()),
        );
        assert!(
            state.any_tool_open() && state.tools.json.window_id.is_some(),
            "打开说明不得关闭独立工具窗"
        );
        assert!(state.settings_open);
        assert_eq!(
            state.plugin_docs_open.as_deref(),
            Some("com.kite.calculator")
        );
    }

    #[test]
    fn json_tool_close_does_not_resize_or_switch_main_window() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenTool(ToolKind::Json));
        let _ = update(&mut state, Message::PluginConfirmTool);
        assert!(state.tools.json.window_id.is_some());
        let _ = update(&mut state, Message::Tool(ToolKind::Json, ToolOp::Close));
        assert!(state.settings_open);
        assert!(!state.hidden);
        assert!(state.tools.json.window_id.is_none());
        assert!(!state.any_tool_open());
    }

    #[test]
    fn calculator_trigger_requires_enter_and_keeps_core_results() {
        use actions::launch_selected;
        let mut state = test_state("=1+2");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(calculator_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged("=1+2".into()));
        assert!(state.provider_mode.is_none());
        let core = SearchResult::scored(
            AppItem::scanned(
                "core:test".into(),
                "本机结果".into(),
                "test".into(),
                None,
                None,
                "app",
            ),
            1000,
            "app",
        );
        state.apply_app_search_ready(
            state.app_query_generation,
            state.query.clone(),
            vec![core],
            0,
            state.index_generation,
        );
        assert_eq!(state.results[0].item.id, "core:test");
        assert!(state
            .results
            .iter()
            .any(|r| r.item.id == "plugin-trigger:com.kite.calculator:calculate"));
        state.selected = state
            .results
            .iter()
            .position(|r| r.item.id == "plugin-trigger:com.kite.calculator:calculate")
            .unwrap();
        let _ = launch_selected(&mut state);
        assert!(
            state
                .provider_mode
                .as_ref()
                .is_some_and(|a| a.provider_id == "calculate"),
            "选中插件入口后进入 Provider"
        );
        assert!(state.pending_tool_confirm.is_none());
        assert!(!state.any_tool_open());
    }

    #[test]
    fn punctuation_trigger_still_shows_plugin_entry() {
        let mut state = test_state("");
        state
            .plugin_registry
            .lock()
            .unwrap()
            .insert_loaded(calculator_plugin());
        let _ = update(&mut state, Message::QueryChanged("=".into()));
        assert!(state.provider_mode.is_none());
        if state.results_stale {
            state.apply_app_search_ready(
                state.app_query_generation,
                state.query.clone(),
                vec![],
                0,
                state.index_generation,
            );
        }
        assert!(state
            .results
            .iter()
            .any(|r| r.item.id == "plugin-trigger:com.kite.calculator:calculate"));
    }

    #[test]
    fn hash_opens_independent_input_and_result_window_after_selection() {
        use actions::launch_selected;
        let mut state = test_state("");
        state
            .plugin_registry
            .lock()
            .unwrap()
            .insert_loaded(devtools_plugin());
        let _ = update(&mut state, Message::QueryChanged("hash abc".into()));
        assert!(state.tools.hash.window_id.is_none());
        state.apply_app_search_ready(
            state.app_query_generation,
            state.query.clone(),
            vec![],
            0,
            state.index_generation,
        );
        state.selected = state
            .results
            .iter()
            .position(|r| r.item.id == "plugin-trigger:com.kite.devtools:hash")
            .unwrap();
        let _ = launch_selected(&mut state);
        assert!(state.tools.hash.window_id.is_some());
        assert!(state.provider_mode.is_none());
        assert_eq!(state.tools.hash.result, hash_tool::sha256_hex("abc"));
        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Hash, ToolOp::PasteReady(Some("kite".into()))),
        );
        assert_eq!(hash_tool::input_text(&state.tools.hash.editor), "kite");
        assert_eq!(state.tools.hash.result, hash_tool::sha256_hex("kite"));
        let _ = update(&mut state, Message::Tool(ToolKind::Hash, ToolOp::Clear));
        assert!(state.tools.hash.result.is_empty());
        let _ = update(&mut state, Message::Tool(ToolKind::Hash, ToolOp::Close));
        assert!(state.tools.hash.window_id.is_none());
    }

    #[test]
    fn clicking_tool_editors_keeps_existing_results() {
        use iced::widget::text_editor::{Action, Content};

        let mut state = test_state("");
        state.tools.json.editor = Content::with_text("{\"a\":1}");
        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Json, ToolOp::Transform { minify: false }),
        );
        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Hash, ToolOp::PasteReady(Some("abc".into()))),
        );
        state.tools.base64.editor = Content::with_text("listary");
        let _ = update(&mut state, Message::Tool(ToolKind::Base64, ToolOp::Encode));
        assert_eq!(state.tools.json.result, "{\n  \"a\": 1\n}");
        assert_eq!(state.tools.hash.result, hash_tool::sha256_hex("abc"));
        assert_eq!(state.tools.base64.result, "bGlzdGFyeQ==");

        let json_result = state.tools.json.result.clone();
        let hash_result = state.tools.hash.result.clone();
        let base64_result = state.tools.base64.result.clone();
        let click = Action::Click(iced::Point::new(0.0, 0.0));

        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Json, ToolOp::Edit(click.clone())),
        );
        assert_eq!(state.tools.json.result, json_result);
        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Hash, ToolOp::Edit(click.clone())),
        );
        assert_eq!(state.tools.hash.result, hash_result);
        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Base64, ToolOp::Edit(click)),
        );
        assert_eq!(state.tools.base64.editor.text(), "listary");
        assert_eq!(state.tools.base64.result, base64_result);

        let _ = update(
            &mut state,
            Message::Tool(
                ToolKind::Base64,
                ToolOp::Edit(Action::Edit(iced::widget::text_editor::Edit::Insert('!'))),
            ),
        );
        assert!(state.tools.base64.result.is_empty());
    }

    #[test]
    fn base64_search_opens_tool_and_supports_both_directions() {
        use actions::launch_selected;
        let mut state = test_state("");
        state
            .plugin_registry
            .lock()
            .unwrap()
            .insert_loaded(devtools_plugin());
        let _ = update(&mut state, Message::QueryChanged("base64 Kite".into()));
        state.apply_app_search_ready(
            state.app_query_generation,
            state.query.clone(),
            vec![],
            0,
            state.index_generation,
        );
        state.selected = state
            .results
            .iter()
            .position(|r| r.item.id == "plugin-trigger:com.kite.devtools:base64")
            .unwrap();
        let _ = launch_selected(&mut state);
        assert!(state.tools.base64.window_id.is_some());
        assert!(state.provider_mode.is_none());
        assert_eq!(state.tools.base64.result, "S2l0ZQ==");

        let _ = update(
            &mut state,
            Message::Tool(
                ToolKind::Base64,
                ToolOp::PasteReady(Some("S2l0ZQ==".into())),
            ),
        );
        let _ = update(&mut state, Message::Tool(ToolKind::Base64, ToolOp::Decode));
        assert_eq!(state.tools.base64.result, "Kite");

        let _ = update(
            &mut state,
            Message::Tool(ToolKind::Base64, ToolOp::PasteReady(Some("bad!".into()))),
        );
        let _ = update(&mut state, Message::Tool(ToolKind::Base64, ToolOp::Decode));
        assert!(state.tools.base64.result.is_empty());
        assert!(state
            .tools
            .base64
            .note
            .as_ref()
            .is_some_and(|(error, _)| *error));

        let _ = update(&mut state, Message::Tool(ToolKind::Base64, ToolOp::Close));
        assert!(state.tools.base64.window_id.is_none());
        assert!(!state.any_tool_open());
    }

    #[test]
    fn base64_settings_entry_keeps_main_window_visible() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenTool(ToolKind::Base64));
        assert!(state.tools.base64.window_id.is_some());
        assert!(state.settings_open);
        assert!(!state.hidden);
        let _ = update(&mut state, Message::Tool(ToolKind::Base64, ToolOp::Close));
        assert!(state.tools.base64.window_id.is_none());
        assert!(state.settings_open);
    }

    #[allow(dead_code)]
    fn _silence_unused(activation: Activation, mode: ResponseMode, action: ResultAction, src: ResultSource) {
        let _ = (activation, mode, action, src);
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;

    fn result(id: &str, target: &str) -> SearchResult {
        SearchResult::scored(
            AppItem::scanned(
                id.to_string(),
                id.to_string(),
                target.to_string(),
                None,
                None,
                "test",
            ),
            1,
            "test",
        )
    }

    #[test]
    fn context_menu_keeps_clicked_item_when_results_reorder() {
        let mut results = vec![
            result("first", r"C:\Apps\first.exe"),
            result("second", r"C:\Apps\second.exe"),
        ];
        let clicked = capture_menu_item(&results, 0).expect("clicked item");
        assert_eq!(clicked.id, "first");

        results.remove(0);
        assert!(capture_menu_item(&results, 0).is_some());
        assert_eq!(clicked.id, "first");
    }

    #[test]
    fn left_click_outside_closes_context_menu() {
        use super::super::test_support::test_state;
        let mut state = test_state("");
        state.cursor.set(iced::Point::new(20.0, 30.0));
        let _ = update(&mut state, Message::ContextMenu(0));
        assert!(state.menu.is_some());

        state.cursor.set(iced::Point::new(600.0, 400.0));
        let _ = update(&mut state, Message::MousePressed(iced::mouse::Button::Left));
        assert!(state.menu.is_none(), "left click outside should close");
    }
}
