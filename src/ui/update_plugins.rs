//! 插件设置与 HostCall：启停、导入、目录、查询落地。

use super::actions::{
    close_settings, flash, launch_and_hide, open_path_and_hide, open_url_and_hide,
    settings_window_task, sync_scroll,
};
use super::{
    plog, plugin, system, with_plugin_host, with_plugin_registry, Message, Section, State,
};
use crate::app;
use iced::Task;

pub(super) fn handle(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::PluginQueryReady(generation, payload) => {
            state.apply_plugin_query_ready(generation, payload);
            if let Some(err) = state.plugin_flash.clone() {
                plog(&format!("plugin query error: {err}"));
                state.qlog(|| format!("plugin query error: {err}"));
            }
            Task::none()
        }
        Message::PluginHostCall(plugin_id, call) => {
            state.qlog(|| format!("plugin host call {plugin_id} {call:?}"));
            use plugin::HostCall;
            match call {
                HostCall::ClipboardWrite(text) => iced::clipboard::write(text),
                HostCall::OpenUrl(url) => open_url_and_hide(state, &url),
                HostCall::OpenPath(path) => open_path_and_hide(state, &path),
                HostCall::HideKite => launch_and_hide(state),
            }
        }
        Message::PluginSetEnabled(id, enabled) => {
            with_plugin_registry(state, |reg| reg.set_enabled(&id, enabled));
            if !enabled {
                with_plugin_host(state, |host| host.reload(&id));
            }
            state.qlog(|| format!("plugin set_enabled id={id} enabled={enabled}"));
            state.refresh_results();
            flash(
                state,
                if enabled {
                    "插件已启用"
                } else {
                    "插件已禁用"
                },
            )
        }
        Message::PluginReload(id) => {
            let result = with_plugin_registry(state, |reg| reg.reload_from_disk(&id));
            state.qlog(|| format!("plugin reload id={id} err={result:?}"));
            with_plugin_host(state, |host| host.reload(&id));
            flash(state, "插件已重新加载")
        }
        Message::PluginOpenDir(id) => {
            let root = with_plugin_registry(state, |reg| reg.get(&id).map(|p| p.root.clone()));
            if let Some(root) = root {
                let r = app::uwp::launch_shell_path(&root.to_string_lossy());
                state.qlog(|| format!("plugin open dir id={id} err={r:?}"));
            }
            Task::none()
        }
        Message::PluginOpenLog(id) => {
            let path = plugin::plugin_log_path(&state.data_dir, &id);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if !path.exists() {
                let _ = std::fs::write(&path, b"");
            }
            let r = app::uwp::launch_shell_path(&path.to_string_lossy());
            state.qlog(|| format!("plugin open log id={id} err={r:?}"));
            Task::none()
        }
        Message::PluginUninstall(id) => {
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            with_plugin_host(state, |host| host.reload(&id));
            let r = with_plugin_registry(state, |reg| reg.uninstall(&id, &plugins_dir));
            state.qlog(|| format!("plugin uninstall id={id} err={r:?}"));
            state.refresh_results();
            flash(state, "插件已卸载")
        }
        Message::PluginImportPathChanged(v) => {
            state.plugin_import_path = v;
            Task::none()
        }
        Message::PluginImportFromPath => {
            let path = state.plugin_import_path.trim().to_string();
            if path.is_empty() {
                return flash(state, "请先粘贴插件文件夹路径");
            }
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            let source = std::path::PathBuf::from(&path);
            let results = plugin::import_from_path(&source, &plugins_dir);
            super::interaction::apply_plugin_import_results(state, results)
        }
        Message::PluginInstallOfficial => {
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            let results = plugin::install_official_plugins(&plugins_dir);
            super::interaction::apply_plugin_import_results(state, results)
        }
        Message::PluginOpenPluginsDir => {
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            let _ = std::fs::create_dir_all(&plugins_dir);
            let r = app::uwp::launch_shell_path(&plugins_dir.to_string_lossy());
            state.qlog(|| format!("plugin open plugins dir err={r:?}"));
            Task::none()
        }
        Message::PluginRescanPlugins => {
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            with_plugin_registry(state, |reg| reg.scan_dir(&plugins_dir));
            state.refresh_results();
            flash(state, "已重新扫描插件目录")
        }
        Message::PluginTryExample(example) => {
            let task = close_settings(state);
            state.plugin_docs_open = None;
            if !example.is_empty() {
                if state.provider_mode.is_some() {
                    state.exit_provider_mode();
                }
                state.query = example;
                state.request_direct_path();
                state.pending_tool_confirm = None;
                state.request_file_search();
                state.refresh_results();
                state.hover_suppressed = false;
            }
            Task::batch([task, sync_scroll(state)])
        }
        Message::PluginToggleDocs(id) => {
            state.settings_open = true;
            state.settings_section = Section::Plugins;
            state.plugin_docs_open = if state.plugin_docs_open.as_deref() == Some(id.as_str()) {
                None
            } else {
                Some(id)
            };
            state
                .window_id
                .map(settings_window_task)
                .unwrap_or_else(Task::none)
        }
        Message::PluginOpenTool(kind) => super::tools::open_from_settings(state, kind),
        Message::PluginConfirmTool => super::tools::confirm_open(state),
        Message::PluginCancelToolConfirm => super::tools::cancel_confirm(state),
        Message::Tool(kind, op) => super::tools::handle_op(state, kind, op),
        Message::ToolWindowClosed(id) => super::tools::handle_window_closed(state, id),
        _ => {
            let _ = system::everything::FileFilter::All;
            Task::none()
        }
    }
}
