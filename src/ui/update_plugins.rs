//! 插件设置与 HostCall：启停、导入、目录、查询落地。

use super::actions::{
    close_settings, flash, launch_and_hide, open_path_and_hide, open_url_and_hide,
    settings_window_task, sync_scroll,
};
use super::{plog, plugin, with_plugin_registry, Message, Section, State};
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
        Message::PluginExecuteReady(plugin_id, generation, result) => {
            if generation != state.plugin_query_generation
                || !state
                    .provider_mode
                    .as_ref()
                    .is_some_and(|activation| activation.plugin_id == plugin_id)
                || !with_plugin_registry(state, |registry| {
                    registry
                        .get(&plugin_id)
                        .is_some_and(|plugin| plugin.enabled)
                })
            {
                return Task::none();
            }
            // Actions can change service status; query again without requiring new input.
            state.refresh_results();
            match result {
                Ok(()) => Task::none(),
                Err(error) => flash(state, &format!("插件操作失败：{error}")),
            }
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
                cancel_plugin_queries(state);
                reload_plugin_in_background(state, id.clone(), "禁用", Ok(()));
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
            cancel_plugin_queries(state);
            let result = with_plugin_registry(state, |reg| reg.reload_from_disk(&id));
            state.qlog(|| format!("plugin reload id={id} err={result:?}"));
            reload_plugin_in_background(state, id, "重新加载", result);
            flash(state, "正在重新加载插件…")
        }
        Message::PluginManagementReady(operation, result) => {
            state.refresh_results();
            match result {
                Ok(()) => flash(state, &format!("插件已{operation}")),
                Err(error) => flash(state, &format!("插件{operation}失败：{error}")),
            }
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
            cancel_plugin_queries(state);
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            with_plugin_registry(state, |reg| reg.set_enabled(&id, false));
            let host = state.plugin_host.clone();
            let registry = state.plugin_registry.clone();
            std::thread::spawn(move || {
                host.lock().unwrap_or_else(|e| e.into_inner()).reload(&id);
                let result = registry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .uninstall(&id, &plugins_dir);
                super::send_event(Message::PluginManagementReady("卸载".into(), result));
            });
            state.refresh_results();
            flash(state, "正在卸载插件…")
        }
        Message::PluginImportReady(results) => {
            super::interaction::apply_plugin_import_results(state, results)
        }
        Message::PluginImportPathChanged(v) => {
            state.plugin_import_path = v;
            Task::none()
        }
        Message::PluginImportFromPath => {
            let path = state.plugin_import_path.trim().to_string();
            if path.is_empty() {
                return flash(state, "请先粘贴插件 ZIP 或文件夹路径");
            }
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            let source = std::path::PathBuf::from(&path);
            if source.is_file() {
                cancel_plugin_queries(state);
                let host = state.plugin_host.clone();
                std::thread::spawn(move || {
                    // Serialize validation/replacement against RPC and stop only
                    // the validated target before replacing its mapped executable.
                    let mut host = host.lock().unwrap_or_else(|error| error.into_inner());
                    let results = plugin::install::import_from_path_with_pre_replace(
                        &source,
                        &plugins_dir,
                        |id| {
                            host.reload(id);
                            Ok(())
                        },
                    );
                    drop(host);
                    super::send_event(Message::PluginImportReady(results));
                });
                return flash(state, "正在导入插件包…");
            }
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
        _ => Task::none(),
    }
}

// Waiting for an in-flight plugin RPC must never block the UI thread.
fn cancel_plugin_queries(state: &mut State) {
    state.plugin_query_generation = state.plugin_query_generation.wrapping_add(1);
    state.plugin_query_worker.cancel_current();
    state.file_query_generation = state.file_query_generation.wrapping_add(1);
    state.file_search_worker.cancel_current();
    state.file_results.clear();
}

pub(super) fn reload_plugin_in_background(
    state: &State,
    id: String,
    operation: &'static str,
    result: Result<(), String>,
) {
    let host = state.plugin_host.clone();
    std::thread::spawn(move || {
        host.lock().unwrap_or_else(|e| e.into_inner()).reload(&id);
        super::send_event(Message::PluginManagementReady(operation.into(), result));
    });
}

#[cfg(test)]
mod management_tests {
    use super::super::test_support::test_state;
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn management_messages_return_while_host_rpc_lock_is_held_and_reload_eventually_runs() {
        for message in [
            Message::PluginSetEnabled("com.kite.loci".into(), false),
            Message::PluginReload("com.kite.loci".into()),
            Message::PluginUninstall("com.kite.loci".into()),
        ] {
            let mut state = test_state("chrome");
            let host = state.plugin_host.clone();
            let (locked_tx, locked_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let holder = std::thread::spawn(move || {
                let mut host = host.lock().unwrap();
                host.note_crash("com.kite.loci", "held RPC fixture");
                locked_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(1));
            });
            locked_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            let started = Instant::now();
            let _ = handle(&mut state, message);
            let elapsed = started.elapsed();
            release_tx.send(()).ok();
            holder.join().unwrap();
            assert!(
                elapsed < Duration::from_millis(250),
                "UI waited for host lock: {elapsed:?}"
            );
            let deadline = Instant::now() + Duration::from_secs(1);
            while state.plugin_host.lock().unwrap().state("com.kite.loci")
                != plugin::PluginRuntimeState::Dormant
            {
                assert!(Instant::now() < deadline, "queued host reload did not run");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn uninstall_disables_registry_immediately_and_deletes_package_after_host_stops() {
        let mut state = test_state("chrome");
        state.data_dir = std::env::temp_dir().join(format!(
            "kite-managed-uninstall-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
        let root = plugins_dir.join("com.kite.loci");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("owned-fixture.txt"), b"fixture").unwrap();
        state
            .plugin_registry
            .lock()
            .unwrap()
            .insert_loaded(plugin::registry::RegisteredPlugin {
                manifest: plugin::manifest::parse_manifest(include_str!(
                    "../plugin/testdata/loci.json"
                ))
                .unwrap(),
                root: root.clone(),
                enabled: true,
                phase: plugin::registry::RuntimePhase::Dormant,
                last_error: None,
            });
        let host = state.plugin_host.clone();
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _host = host.lock().unwrap();
            locked_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(1));
        });
        locked_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let _ = handle(&mut state, Message::PluginUninstall("com.kite.loci".into()));
        assert!(
            !state
                .plugin_registry
                .lock()
                .unwrap()
                .get("com.kite.loci")
                .unwrap()
                .enabled
        );
        assert!(
            root.exists(),
            "package must remain until host shutdown finishes"
        );
        release_tx.send(()).ok();
        holder.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while state
            .plugin_registry
            .lock()
            .unwrap()
            .get("com.kite.loci")
            .is_some()
        {
            assert!(Instant::now() < deadline, "queued uninstall did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!root.exists());
        std::fs::remove_dir_all(&state.data_dir).unwrap();
    }

    #[test]
    fn action_completion_refreshes_status_and_reports_failure_only_for_current_provider() {
        let mut state = test_state("loci status");
        state
            .plugin_registry
            .lock()
            .unwrap()
            .insert_loaded(plugin::registry::RegisteredPlugin {
                manifest: plugin::manifest::parse_manifest(include_str!(
                    "../plugin/testdata/loci.json"
                ))
                .unwrap(),
                root: std::env::temp_dir(),
                enabled: true,
                phase: plugin::registry::RuntimePhase::Dormant,
                last_error: None,
            });
        state.provider_mode =
            plugin::route_query("loci status", &state.plugin_registry.lock().unwrap());
        let generation = state.plugin_query_generation;
        let _ = handle(
            &mut state,
            Message::PluginExecuteReady(
                "com.kite.loci".into(),
                generation,
                Err("Missing bundled installer".into()),
            ),
        );
        assert!(
            state.plugin_query_generation > generation,
            "status must be queried again without typing"
        );
        assert!(state
            .flash
            .as_deref()
            .unwrap()
            .contains("Missing bundled installer"));
        let current = state.plugin_query_generation;
        state.flash = None;
        let _ = handle(
            &mut state,
            Message::PluginExecuteReady(
                "com.kite.loci".into(),
                generation,
                Err("stale error".into()),
            ),
        );
        assert_eq!(state.plugin_query_generation, current);
        assert!(state.flash.is_none());
        let _ = handle(
            &mut state,
            Message::PluginExecuteReady("different.plugin".into(), current, Ok(())),
        );
        assert_eq!(state.plugin_query_generation, current);
        state
            .plugin_registry
            .lock()
            .unwrap()
            .set_enabled("com.kite.loci", false);
        let _ = handle(
            &mut state,
            Message::PluginExecuteReady("com.kite.loci".into(), current, Ok(())),
        );
        assert_eq!(state.plugin_query_generation, current);
    }

    #[test]
    fn management_completion_reports_failure() {
        let mut state = test_state("");
        let _ = handle(
            &mut state,
            Message::PluginManagementReady("卸载".into(), Err("permission denied".into())),
        );
        assert!(state
            .flash
            .as_deref()
            .is_some_and(|text| text.contains("卸载失败") && text.contains("permission denied")));
    }
}
