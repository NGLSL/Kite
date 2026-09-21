//! 设置页消息：开关、搜索引擎、别名、便携目录、热键录制入口。

use super::actions::{close_settings, flash, load_aliases, open_settings, refresh_after_alias_change};
use super::interaction::{persist_portable_dirs, request_rescan};
use super::{search, system, Message, Section, State};
use crate::storage::settings::UserAlias;
use crate::system::hotkey::parse_raw;
use iced::Task;

pub(super) fn handle(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::OpenSettings => open_settings(state),
        Message::CloseSettings => {
            end_hotkey_record(state);
            close_settings(state)
        }
        Message::SettingsSection(s) => {
            if s != Section::Plugins {
                state.plugin_docs_open = None;
            }
            state.settings_section = s;
            Task::none()
        }
        Message::SetAutostart(v) => {
            state.autostart = v;
            if let Some(db) = &mut state.history {
                let _ = db.save_setting("autostart", if v { "1" } else { "0" });
            }
            let r = system::autostart::set_autostart(v);
            state.qlog(|| format!("autostart -> {v} err={r:?}"));
            flash(state, "设置已保存")
        }
        Message::SetHideOnBlur(v) => {
            state.hide_on_blur = v;
            if let Some(db) = &mut state.history {
                let _ = db.save_setting("hide_on_blur", if v { "1" } else { "0" });
            }
            flash(state, "设置已保存")
        }
        Message::SetHistoryRecording(v) => {
            state.history_recording = v;
            if let Some(db) = &mut state.history {
                let _ = db.save_setting("history_recording", if v { "1" } else { "0" });
            }
            flash(state, "设置已保存")
        }
        Message::SetQueryLog(v) => {
            state.query_log = v;
            if let Some(db) = &mut state.history {
                let _ = db.save_setting("query_log", if v { "1" } else { "0" });
            }
            flash(state, "设置已保存")
        }
        Message::SetThemeMode(mode) => {
            state.theme_mode = mode;
            if let Some(db) = &mut state.history {
                let _ = db.save_theme_mode(mode.as_str());
            }
            flash(state, "外观模式已切换")
        }
        Message::SetSearchEngine(id) => {
            state.search_engine = id.clone();
            if let Some(preset) = system::search_engine::preset_by_id(&id) {
                if !preset.template.is_empty() {
                    state.search_engine_custom = preset.template.to_string();
                }
            }
            if let Some(db) = &mut state.history {
                let _ = db.set_search_engine(&id);
                match id.as_str() {
                    "auto" => {
                        let _ = db.save_setting("search_url_template", "");
                    }
                    "custom" => {
                        if system::search_engine::is_valid_template(&state.search_engine_custom) {
                            let _ = db.set_search_url_template(state.search_engine_custom.trim());
                        }
                    }
                    _ => {
                        if let Some(t) = system::search_engine::preset_by_id(&id)
                            .map(|p| p.template)
                            .filter(|t| !t.is_empty())
                        {
                            let _ = db.set_search_url_template(t);
                        }
                    }
                }
            }
            flash(state, "搜索引擎已更新")
        }
        Message::SearchEngineCustomChanged(template) => {
            state.search_engine_custom = template.clone();
            if state.search_engine == "custom"
                && system::search_engine::is_valid_template(&template)
            {
                if let Some(db) = &mut state.history {
                    let _ = db.set_search_url_template(template.trim());
                }
            }
            Task::none()
        }
        Message::SetWebSearchHotkey(spec) => {
            let normalized = system::hotkey::normalize_web_search_hotkey(&spec).to_string();
            state.web_search_hotkey = normalized.clone();
            if let Some(db) = &mut state.history {
                let _ = db.set_web_search_hotkey(&normalized);
            }
            flash(state, "网页搜索快捷键已更新")
        }
        Message::ClearHistory => {
            if let Some(db) = &mut state.history {
                let _ = db.clear_history();
            }
            state.invalidate_prefs_cache();
            state.refresh_results();
            flash(state, "使用历史已清空")
        }
        Message::ApplyHotkey(spec) => {
            if parse_raw(&spec).is_none() {
                return flash(state, "无法解析该快捷键");
            }
            end_hotkey_record(state);
            match super::HOTKEY_CMD
                .get()
                .map(|tx| tx.send(super::HotkeyCmd::Apply(spec)))
            {
                Some(Ok(())) => flash(state, "正在应用快捷键…"),
                _ => flash(state, "快捷键服务不可用，请重启 Kite"),
            }
        }
        Message::HotkeyRecordCaptured(spec) => {
            state.hotkey_recording = false;
            match super::HOTKEY_CMD.get().map(|tx| {
                let _ = tx.send(super::HotkeyCmd::EndRecord);
                tx.send(super::HotkeyCmd::Apply(spec))
            }) {
                Some(Ok(())) => flash(state, "正在应用快捷键…"),
                _ => flash(state, "快捷键服务不可用，请重启 Kite"),
            }
        }
        Message::AliasInputChanged(s) => {
            state.alias_input = s;
            Task::none()
        }
        Message::AliasTargetChanged(s) => {
            state.alias_target_input = s.clone();
            state.alias_pick = None;
            state.alias_candidates = if s.trim().is_empty() {
                Vec::new()
            } else {
                let index = state.index.lock().unwrap_or_else(|e| e.into_inner());
                search::name_candidates(&index.apps, &s, 5)
            };
            Task::none()
        }
        Message::AliasPick(i) => {
            state.alias_pick = state.alias_candidates.get(i).map(|c| UserAlias {
                alias: String::new(),
                target_name: c.item.display_name.clone(),
                target_id: Some(c.item.id.clone()),
            });
            Task::none()
        }
        Message::AliasAdd => {
            let alias = state.alias_input.trim().to_string();
            if alias.is_empty() {
                return flash(state, "请填写别名");
            }
            let Some(pick) = state.alias_pick.clone() else {
                return flash(state, "请先从候选中选择目标");
            };
            if let Some(db) = &mut state.history {
                let _ = db.set_alias(&alias, pick.target_id.as_deref(), &pick.target_name);
            }
            state.alias_input.clear();
            state.alias_target_input.clear();
            state.alias_candidates.clear();
            state.alias_pick = None;
            load_aliases(state);
            refresh_after_alias_change(state);
            flash(state, "别名已保存")
        }
        Message::AliasRemove(alias) => {
            if let Some(db) = &mut state.history {
                let _ = db.remove_alias(&alias);
            }
            load_aliases(state);
            refresh_after_alias_change(state);
            flash(state, "别名已删除")
        }
        Message::PortableDirInputChanged(value) => {
            state.portable_dir_input = value;
            Task::none()
        }
        Message::AddPortableDir => {
            let raw = state.portable_dir_input.trim().trim_matches('"');
            let path = std::path::PathBuf::from(raw);
            if raw.is_empty() || !path.is_dir() {
                return flash(state, "请输入已经存在的软件目录");
            }
            let key = crate::app::scanner::util::normalize_path_key(raw);
            if state
                .portable_dirs
                .iter()
                .any(|dir| crate::app::scanner::util::normalize_path_key(dir) == key)
            {
                return flash(state, "该目录已经加入索引");
            }
            state.portable_dirs.push(path.to_string_lossy().to_string());
            persist_portable_dirs(state);
            state.portable_dir_input.clear();
            request_rescan(state);
            flash(state, "目录已加入，正在重新扫描")
        }
        Message::RemovePortableDir(index) => {
            if index >= state.portable_dirs.len() {
                return Task::none();
            }
            state.portable_dirs.remove(index);
            persist_portable_dirs(state);
            request_rescan(state);
            flash(state, "目录已移除，正在重新扫描")
        }
        Message::StartHotkeyRecord => {
            end_hotkey_record(state);
            state.hotkey_recording = true;
            if let Some(tx) = super::HOTKEY_CMD.get() {
                let _ = tx.send(super::HotkeyCmd::BeginRecord);
            }
            flash(state, "请按下新的快捷键（Esc 取消）")
        }
        _ => Task::none(),
    }
}

/// 退出录制态：清标志并通知热键线程注销吞键、恢复全局快捷键。
pub(super) fn end_hotkey_record(state: &mut State) {
    if !state.hotkey_recording {
        return;
    }
    state.hotkey_recording = false;
    if let Some(tx) = super::HOTKEY_CMD.get() {
        let _ = tx.send(super::HotkeyCmd::EndRecord);
    }
}
