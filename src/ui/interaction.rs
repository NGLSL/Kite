//! Iced 消息到界面状态的更新。

use super::*;

pub(super) fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Hotkey(t0) => {
            plog(&format!(
                "hotkey recv->update {}us",
                t0.elapsed().as_micros()
            ));
            match state.window_id {
                Some(_) if state.hidden => show_launcher(state),
                Some(id) => {
                    plog("hide issued (hotkey toggle)");
                    hide(state);
                    window::set_mode(id, window::Mode::Hidden)
                }
                None => {
                    plog("hotkey before window ready; ignored");
                    Task::none()
                }
            }
        }
        Message::EnsureVisible => {
            if state.hidden {
                plog("ensure visible from secondary instance");
                show_launcher(state)
            } else if let Some(id) = state.window_id {
                plog("secondary activate while visible; focus only");
                window::gain_focus(id)
            } else {
                Task::none()
            }
        }
        Message::KeyPressed(key, physical, mods) => {
            // 本地跟踪 Alt（SYSKEY 下 modifiers.alt() 可能为假）
            if matches!(key, Key::Named(Named::Alt)) {
                state.alt_down = true;
                state.query_at_alt = Some(state.query.clone());
                state.alt_digit_consumed = false;
                state.qlog(|| "alt down".to_string());
            }
            on_key(state, key, physical, mods)
        }
        Message::KeyReleased(key, _mods) => {
            if matches!(key, Key::Named(Named::Alt)) {
                state.alt_down = false;
                state.query_at_alt = None;
                state.alt_digit_consumed = false;
                state.qlog(|| "alt up".to_string());
            }
            Task::none()
        }
        Message::Composing(active) => {
            if state.ime_composing != active {
                state.qlog(|| format!("ime composing={active}"));
                state.ime_composing = active;
            }
            Task::none()
        }
        Message::ImeCommit(text) => {
            state.qlog(|| format!("ime commit '{text}'"));
            Task::none()
        }
        Message::WindowBlur => {
            // 设置页 / JSON 工具窗打开时主窗失焦不隐藏（工具是独立窗，不改主窗）
            if state.settings_open || state.plugin_tool_open {
                return Task::none();
            }
            if !state.hidden {
                plog("hide issued (blur)");
                hide(state);
                hide_task(state)
            } else {
                Task::none()
            }
        }
        Message::WindowReady(id) => {
            plog(&format!("window ready id={:?}", id.map(|i| i.to_string())));
            if state.window_id.is_none() {
                state.window_id = id;
            }
            // 主窗口就绪：工具窗独立，不在主窗上切尺寸/视图。
            if state.settings_open {
                id.map(settings_window_task).unwrap_or_else(Task::none)
            } else {
                Task::none()
            }
        }
        Message::JsonToolDrag => state
            .json_tool_window
            .map(window::drag)
            .unwrap_or_else(Task::none),
        Message::JsonToolWindowClosed(id) => {
            if state.json_tool_window == Some(id) {
                state.json_tool_window = None;
                state.plugin_tool_open = false;
                state.qlog(|| "json tool window closed".to_owned());
            }
            Task::none()
        }
        Message::QueryChanged(q) => {
            if state.hidden {
                return Task::none();
            }
            // 输入框先收到 Alt+数字时，直接走同一启动动作；不能只吞掉数字。
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
            // `json` / `json {...}`：非内联工具，只进二次确认，不直接开窗。
            if let Some(task) = try_open_json_tool_from_query(state) {
                return task;
            }
            // 查询不再命中 json 时，丢掉搜索态待确认。
            if state.pending_tool_confirm.as_ref().is_some_and(|p| !p.from_settings) {
                state.pending_tool_confirm = None;
            }
            state.request_file_search();
            state.refresh_results();
            state.hover_suppressed = false;
            sync_scroll(state)
        }
        Message::ClearQuery => {
            state.query.clear();
            state.request_file_search();
            state.refresh_results();
            // × 按钮会抢走键盘焦点，清空后还给输入框
            Task::batch([
                iced::widget::operation::focus(state.input_id.clone()),
                sync_scroll(state),
            ])
        }
        Message::HoverSelect(i) => {
            // 对齐前端 onMouseMove：悬停只改选中，不滚动。
            // 键盘导航/scroll_to 后抑制，避免列表内容换行触发 on_enter 抢选中。
            if state.hover_suppressed {
                return Task::none();
            }
            if state.selected != i {
                state.selected = i;
            }
            Task::none()
        }
        Message::LaunchIndex(i) => {
            state.menu = None;
            state.selected = i;
            launch_selected(state)
        }
        Message::ToggleFiles => {
            state.files_mode = !state.files_mode;
            state.qlog(|| format!("files toggle -> {}", state.files_mode));
            state.request_file_search();
            state.refresh_results();
            Task::none()
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
        Message::AppSearchReady(generation, query, hits, elapsed_us, index_generation) => {
            state.apply_app_search_ready(generation, query, hits, elapsed_us, index_generation);
            sync_scroll(state)
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
            let index = state.index.clone();
            let dir = state.icon_dir.clone();
            let options = state.scan_options.clone();
            let tx = EVENT_TX.get().expect("event tx").clone();
            backend::request_build(index, dir, options, tx);
            flash(state, "正在重新扫描应用索引…")
        }
        Message::Quit => {
            state.qlog(|| "quit requested".to_owned());
            std::process::exit(0);
        }
        Message::FlashClear => {
            state.flash = None;
            Task::none()
        }
        Message::StartHotkeyRecord => {
            state.hotkey_recording = true;
            flash(state, "请按下新的快捷键（Esc 取消）")
        }
        Message::CheckUpdate => {
            if state.update_checking {
                return Task::none();
            }
            state.update_checking = true;
            state.update_status = None;
            state.update_asset = None;
            std::thread::spawn(|| {
                let result = system::update::check_latest(env!("CARGO_PKG_VERSION"));
                let _ = EVENT_TX
                    .get()
                    .expect("event tx")
                    .unbounded_send(Message::UpdateResult(result));
            });
            Task::none()
        }
        Message::UpdateResult(r) => {
            state.update_checking = false;
            state.update_asset = None;
            state.update_status = Some(match r {
                Ok(system::update::CheckResult::UpToDate) => Ok("latest".to_string()),
                Ok(system::update::CheckResult::Available { tag, installer }) => {
                    state.update_asset = installer;
                    Ok(tag)
                }
                Err(e) => Err(e),
            });
            Task::none()
        }
        Message::UpdateInstallerLaunched => {
            state.update_checking = false;
            flash(state, "安装器已打开，请按提示完成安装")
        }
        Message::DownloadUpdate => {
            if state.update_checking {
                return Task::none();
            }
            let Some(asset) = state.update_asset.clone() else {
                return flash(state, "没有可用的更新下载地址");
            };
            state.update_checking = true;
            std::thread::spawn(move || {
                let result = system::update::download_verified(&asset).and_then(|path| {
                    plog(&format!("update installer verified path={path:?}"));
                    app::uwp::launch_runas(&path.to_string_lossy(), "", None)
                });
                if result.is_ok() {
                    plog("update installer launched");
                    let _ = EVENT_TX
                        .get()
                        .expect("event tx")
                        .unbounded_send(Message::UpdateInstallerLaunched);
                    return;
                }
                plog(&format!(
                    "update automatic install failed: {:?}",
                    result.err()
                ));
                let fallback = app::uwp::launch_shell_path(system::update::LATEST_RELEASE_URL);
                plog(&format!("update fallback releases err={fallback:?}"));
                let error = if fallback.is_ok() {
                    "自动安装失败，已打开 GitHub 最新发布页，请手动安装".to_string()
                } else {
                    "自动安装失败，打开 GitHub 最新发布页也失败".to_string()
                };
                let _ = EVENT_TX
                    .get()
                    .expect("event tx")
                    .unbounded_send(Message::UpdateResult(Err(error)));
            });
            Task::none()
        }
        Message::OpenReleases => {
            let r = app::uwp::launch_shell_path(system::update::LATEST_RELEASE_URL);
            state.qlog(|| format!("open releases err={r:?}"));
            if r.is_err() {
                flash(state, "打开 GitHub 最新发布页失败")
            } else {
                Task::none()
            }
        }
        Message::OpenRepository => {
            let r = app::uwp::launch_shell_path(system::update::REPOSITORY_URL);
            state.qlog(|| format!("open repository err={r:?}"));
            if r.is_err() {
                flash(state, "打开 Kite 仓库失败")
            } else {
                Task::none()
            }
        }
        Message::HotkeyUnavailable(spec) => {
            state.qlog(|| format!("hotkey unavailable: {spec}"));
            let show = open_settings(state);
            state.settings_section = Section::Hotkey;
            Task::batch([
                show,
                flash(state, &format!("快捷键 {spec} 已被占用，请更换组合键")),
            ])
        }
        Message::HotkeyRegistrationResult(spec, result) => match result {
            Ok(()) => {
                if let Some(db) = &mut state.history {
                    let _ = db.save_setting("hotkey", &spec);
                }
                state.hotkey = spec.clone();
                state.hotkey_label = system::hotkey::display_label(&spec);
                flash(state, "快捷键已更新")
            }
            Err(error) => {
                state.qlog(|| format!("hotkey change rejected: {spec}; error={error}"));
                flash(state, &format!("快捷键 {spec} 无法使用，请换一个组合键"))
            }
        },
        Message::OpenSettings => open_settings(state),
        Message::CloseSettings => close_settings(state),
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
        // 只切换日志闸门：不重跑搜索、不动代际、不失效缓存，
        // 因此开关前后结果、顺序与缓存行为完全一致。
        Message::SetQueryLog(v) => {
            state.query_log = v;
            if let Some(db) = &mut state.history {
                let _ = db.save_setting("query_log", if v { "1" } else { "0" });
            }
            flash(state, "设置已保存")
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
                            let _ = db
                                .set_search_url_template(state.search_engine_custom.trim());
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
            state.refresh_results();
            flash(state, "使用历史已清空")
        }
        Message::ApplyHotkey(spec) => {
            if parse_raw(&spec).is_none() {
                return flash(state, "无法解析该快捷键");
            }
            match HOTKEY_CMD.get().map(|tx| tx.send(spec)) {
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
            let key = app::scanner::util::normalize_path_key(raw);
            if state
                .portable_dirs
                .iter()
                .any(|dir| app::scanner::util::normalize_path_key(dir) == key)
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
        Message::CursorMoved(p) => {
            state.cursor.set(p);
            // 鼠标有实际位移才恢复悬停选中；scroll_to 造成的 on_enter 不算。
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
            // 右键交给 ContextMenu 做开关切换；这里只处理左键点菜单外关闭，
            // 否则第二次右键会先被关掉、又被 ContextMenu 立刻重新打开。
            if btn != iced::mouse::Button::Left {
                return Task::none();
            }
            // 前端行为：菜单开着时点击菜单外任意位置 → 关闭（点在菜单内交给菜单项按钮）
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
            // 第二次右键同一列表：关闭已打开的菜单（toggle）
            if state.menu.is_some() {
                state.menu = None;
                state.qlog(|| "ctx menu closed (re-right-click)".to_owned());
                return Task::none();
            }
            let c = state.cursor.get();
            let x = c.x.min(640.0 - 200.0).max(8.0);
            let y = c.y.min(420.0 - 180.0).max(8.0);
            state.menu = capture_menu_item(&state.results, i).map(|item| (item, x, y));
            Task::none()
        }
        Message::MenuAction(item, action) => menu_action(state, item, action),
        Message::DragWindow => state.window_id.map(window::drag).unwrap_or_else(Task::none),
        Message::FullIndexReady(n) => {
            plog(&format!("full index ready n={n}"));
            state.index_ready = true;
            // 可见时不打断；用户主动重扫则立即采用，避免「扫描完成」与列表不一致。
            if state.hidden || state.rescan_pending {
                apply_pending_full(state);
            }
            if std::mem::take(&mut state.rescan_pending) {
                flash(state, &format!("扫描完成，共 {n} 条"))
            } else {
                Task::none()
            }
        }
        Message::BootstrapReady(n) => {
            plog(&format!("bootstrap index ready n={n}"));
            state.index_ready = true;
            // Bootstrap 直接替换了 state.index：必须作废 query/base 缓存代际，
            // 否则 Bootstrap 前基于 empty index 的结果可能被当成当前代。
            state.index_generation = state.index_generation.wrapping_add(1);
            state.base_hit_cache.clear();
            state.refresh_results();
            Task::none()
        }
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
                HostCall::OpenUrl(url) => {
                    if !plugin::plugin_url_allowed(&url) {
                        return flash(state, "链接无效");
                    }
                    system::env::refresh_process_env();
                    match app::uwp::launch_shell_path(&url) {
                        Ok(()) => {
                            hide(state);
                            hide_task(state)
                        }
                        Err(_) => flash(state, "无法打开链接"),
                    }
                }
                HostCall::OpenPath(path) => {
                    if !plugin::plugin_path_allowed(&path) {
                        return flash(state, "路径无效");
                    }
                    system::env::refresh_process_env();
                    match app::uwp::launch_shell_path(&path) {
                        Ok(()) => {
                            hide(state);
                            hide_task(state)
                        }
                        Err(_) => flash(state, "无法打开该路径"),
                    }
                }
                HostCall::HideKite => {
                    hide(state);
                    hide_task(state)
                }
            }
        }
        Message::PluginSetEnabled(id, enabled) => {
            {
                let mut reg = state.plugin_registry.lock().unwrap_or_else(|e| e.into_inner());
                reg.set_enabled(&id, enabled);
            }
            if !enabled {
                let mut host = state.plugin_host.lock().unwrap_or_else(|e| e.into_inner());
                host.reload(&id);
            }
            state.qlog(|| format!("plugin set_enabled id={id} enabled={enabled}"));
            state.refresh_results();
            flash(state, if enabled { "插件已启用" } else { "插件已禁用" })
        }
        Message::PluginReload(id) => {
            {
                let mut reg = state.plugin_registry.lock().unwrap_or_else(|e| e.into_inner());
                let result = reg.reload_from_disk(&id);
                state.qlog(|| format!("plugin reload id={id} err={result:?}"));
            }
            {
                let mut host = state.plugin_host.lock().unwrap_or_else(|e| e.into_inner());
                host.reload(&id);
            }
            flash(state, "插件已重新加载")
        }
        Message::PluginOpenDir(id) => {
            let root = {
                let reg = state.plugin_registry.lock().unwrap_or_else(|e| e.into_inner());
                reg.get(&id).map(|p| p.root.clone())
            };
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
            {
                let mut host = state.plugin_host.lock().unwrap_or_else(|e| e.into_inner());
                host.reload(&id);
            }
            {
                let mut reg = state.plugin_registry.lock().unwrap_or_else(|e| e.into_inner());
                let r = reg.uninstall(&id, &plugins_dir);
                state.qlog(|| format!("plugin uninstall id={id} err={r:?}"));
            }
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
            apply_plugin_import_results(state, results)
        }
        Message::PluginInstallOfficial => {
            let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
            let results = plugin::install_official_plugins(&plugins_dir);
            apply_plugin_import_results(state, results)
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
            {
                let mut reg = state.plugin_registry.lock().unwrap_or_else(|e| e.into_inner());
                reg.scan_dir(&plugins_dir);
            }
            state.refresh_results();
            flash(state, "已重新扫描插件目录")
        }
        Message::PluginTryExample(example) => {
            // 设置页「试用 / 回到搜索」：关掉设置；json 示例进二次确认，不直接开窗。
            let mut task = close_settings(state);
            state.plugin_docs_open = None;
            if !example.is_empty() {
                state.query = example;
                if let Some(t) = try_open_json_tool_from_query(state) {
                    return Task::batch([task, t]);
                }
                state.pending_tool_confirm = None;
                state.request_file_search();
                state.refresh_results();
                state.hover_suppressed = false;
            }
            task = Task::batch([task, sync_scroll(state)]);
            task
        }
        Message::PluginToggleDocs(id) => {
            // 「说明」在主窗设置页展示；JSON 工具窗保持独立，不互相关闭。
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
        Message::PluginOpenJsonTool => open_json_tool_panel(state),
        Message::PluginConfirmJsonTool => confirm_json_tool_open(state),
        Message::PluginCancelJsonToolConfirm => cancel_json_tool_confirm(state),
        Message::PluginCloseJsonTool => close_json_tool_panel(state),
        Message::JsonToolEdit(action) => {
            state.json_editor.perform(action);
            Task::none()
        }
        Message::JsonToolFormat => apply_json_tool(state, false),
        Message::JsonToolMinify => apply_json_tool(state, true),
        Message::JsonToolPaste => iced::clipboard::read().map(Message::JsonToolPasteReady),
        Message::JsonToolPasteReady(text) => {
            match text {
                Some(raw) if !raw.trim().is_empty() => {
                    state.json_editor = iced::widget::text_editor::Content::with_text(&raw);
                    state.json_result.clear();
                    state.json_tool_note = None;
                }
                _ => {
                    state.json_tool_note = Some((true, "剪贴板为空".into()));
                }
            }
            Task::none()
        }
        Message::JsonToolCopyResult => {
            if state.json_result.trim().is_empty() {
                state.json_tool_note = Some((true, "右侧没有可复制的结果".into()));
                Task::none()
            } else {
                state.json_tool_note = Some((false, "已复制结果".into()));
                iced::clipboard::write(state.json_result.clone())
            }
        }
        Message::JsonToolClear => {
            state.json_editor = iced::widget::text_editor::Content::default();
            state.json_result.clear();
            state.json_tool_note = None;
            Task::none()
        }
    }
}

/// JSON 双栏工具：左侧原文解析后写入右侧（pretty 或 minify）。
fn apply_json_tool(state: &mut State, minify: bool) -> Task<Message> {
    let raw = state.json_editor.text();
    match super::json_tool::transform_json(&raw, minify) {
        Ok(out) => {
            state.json_result = out;
            state.json_tool_note = Some((
                false,
                if minify {
                    "已压缩".into()
                } else {
                    "已格式化".into()
                },
            ));
        }
        Err(msg) => {
            if msg.contains("无效") || msg.contains("序列化") {
                state.json_result.clear();
            }
            state.json_tool_note = Some((true, msg));
        }
    }
    Task::none()
}

/// 导入结果落地：Registry upsert + Host reload + 刷新搜索。
fn apply_plugin_import_results(
    state: &mut State,
    results: Vec<Result<plugin::ImportOutcome, String>>,
) -> Task<Message> {
    let plugins_dir = plugin::default_plugins_dir(&state.data_dir);
    let mut ok = 0usize;
    let mut replaced = 0usize;
    let mut errs: Vec<String> = Vec::new();
    for r in results {
        match r {
            Ok(out) => {
                {
                    let mut host = state.plugin_host.lock().unwrap_or_else(|e| e.into_inner());
                    host.reload(&out.plugin_id);
                }
                {
                    let mut reg = state.plugin_registry.lock().unwrap_or_else(|e| e.into_inner());
                    match plugin::apply_import_to_registry(&mut reg, &out.dest) {
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
            }
            Err(e) => {
                state.qlog(|| format!("plugin import err={e}"));
                errs.push(e);
            }
        }
    }
    let _ = &plugins_dir;
    state.refresh_results();
    if ok == 0 {
        let msg = errs
            .first()
            .cloned()
            .unwrap_or_else(|| "导入失败".into());
        flash(state, &msg)
    } else if errs.is_empty() {
        flash(
            state,
            &format!("已导入 {ok} 个插件{}", if replaced > 0 { "（含覆盖）" } else { "" }),
        )
    } else {
        flash(state, &format!("导入 {ok} 个成功，{} 个失败", errs.len()))
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

        let _ = on_key(
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
        // 使用仓库 resources 作为官方样例源
        crate::system::resources::init(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        // 导入到 state.data_dir（temp）下的 plugins
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
        std::fs::write(
            src.join("plugin.json"),
            serde_json::json!({
                "schema_version": 1,
                "plugin": {
                    "id": "com.kite.frompath",
                    "name": "FromPath",
                    "version": "0.1.0",
                    "description": "测试导入插件：输入 = 触发"
                },
                "compatibility": { "plugin_api": 1 },
                "runtime": { "command": "run.cmd" },
                "contributes": {
                    "examples": ["=1"],
                    "commands": [],
                    "providers": [{
                        "id": "p", "response_mode": "panel",
                        "triggers": [{ "type": "prefix", "value": "=" }]
                    }]
                }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(src.join("run.cmd"), b"@echo off\n").unwrap();
        state.plugin_import_path = src.to_string_lossy().to_string();
        let _ = update(&mut state, Message::PluginImportFromPath);
        let reg = state.plugin_registry.lock().unwrap();
        assert!(reg.get("com.kite.frompath").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stale_list_blocks_enter_launch() {
        let mut state = test_state("win kite");
        state.results_stale = true;
        state.results = vec![SearchResult::scored(
            AppItem::scanned(
                "plugin:com.kite.window-switcher:switch:window-1".into(),
                "Kite".into(),
                "plugin".into(),
                None,
                None,
                "plugin",
            ),
            900,
            "plugin-list",
        )
        .with_source_action(
            ResultSource::Plugin {
                plugin_id: "com.kite.window-switcher".into(),
                provider_id: "switch".into(),
            },
            ResultAction::Plugin {
                plugin_id: "com.kite.window-switcher".into(),
                action_id: "activate_window".into(),
                payload: serde_json::json!({"hwnd": "window-1"}),
            },
        )];
        log::test_records();
        let _ = launch_selected(&mut state);
        let records = log::test_records();
        assert!(
            records.iter().any(|r| r.contains("previous query")),
            "代际过期时列表不得启动，实际：{records:?}"
        );
    }

    #[test]
    fn panel_default_native_action_bypasses_stale_list_gate() {
        let mut state = test_state("=1+2");
        state.results_stale = true;
        state.provider_mode = Some(Activation {
            plugin_id: "com.kite.calculator".into(),
            provider_id: "calculate".into(),
            effective_query: "1+2".into(),
            raw_query: "=1+2".into(),
            response_mode: ResponseMode::Panel,
        });
        state.plugin_panel = Some(
            plugin::parse_panel(&serde_json::json!({
                "blocks": [
                    { "type": "text", "text": "1+2", "style": "secondary" },
                    { "type": "value", "value": "3" }
                ],
                "actions": [{
                    "id": "copy",
                    "label": "复制结果",
                    "default": true,
                    "action": { "type": "copy_text", "text": "3" }
                }]
            }))
            .unwrap(),
        );
        log::test_records();
        let _ = launch_selected(&mut state);
        let records = log::test_records();
        assert!(
            records.iter().any(|r| r.contains("panel native copy_text")),
            "Panel 默认动作应由 Kite Native 复制，不走 plugin/execute"
        );
        assert!(
            !records.iter().any(|r| r.contains("previous query")),
            "Panel 默认动作不应被列表代际闸门拦截"
        );
    }

    #[test]
    fn plugin_manager_enable_disable_touches_registry_and_host() {
        let mut state = test_state("calc");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(calculator_plugin());
        }
        let _ = update(
            &mut state,
            Message::PluginSetEnabled("com.kite.calculator".into(), false),
        );
        {
            let reg = state.plugin_registry.lock().unwrap();
            let p = reg.get("com.kite.calculator").unwrap();
            assert!(!p.enabled);
            assert_eq!(p.phase, RuntimePhase::Disabled);
        }
        let _ = update(
            &mut state,
            Message::PluginSetEnabled("com.kite.calculator".into(), true),
        );
        {
            let reg = state.plugin_registry.lock().unwrap();
            assert!(reg.get("com.kite.calculator").unwrap().enabled);
        }
    }

    #[test]
    fn plugin_query_ready_applies_list_and_drops_stale_generation() {
        let mut state = test_state("win");
        state.provider_mode = Some(Activation {
            plugin_id: "com.kite.window-switcher".into(),
            provider_id: "switch".into(),
            effective_query: "kite".into(),
            raw_query: "win kite".into(),
            response_mode: ResponseMode::List,
        });
        state.plugin_query_generation = 3;
        let item = SearchResult::scored(
            AppItem::scanned(
                "plugin:com.kite.window-switcher:switch:w1".into(),
                "Kite".into(),
                "plugin".into(),
                None,
                None,
                "plugin",
            ),
            900,
            "plugin-list",
        )
        .with_source_action(
            ResultSource::Plugin {
                plugin_id: "com.kite.window-switcher".into(),
                provider_id: "switch".into(),
            },
            ResultAction::Plugin {
                plugin_id: "com.kite.window-switcher".into(),
                action_id: "activate_window".into(),
                payload: serde_json::json!({"hwnd": "w1"}),
            },
        );
        let _ = update(
            &mut state,
            Message::PluginQueryReady(
                3,
                PluginQueryPayload::List {
                    plugin_id: "com.kite.window-switcher".into(),
                    provider_id: "switch".into(),
                    items: vec![item],
                },
            ),
        );
        assert_eq!(state.results.len(), 1);
        assert!(!state.results_stale);

        // 过期代际不覆盖
        let _ = update(
            &mut state,
            Message::PluginQueryReady(
                2,
                PluginQueryPayload::Error("stale".into()),
            ),
        );
        assert_eq!(state.results.len(), 1);
        assert!(state.plugin_flash.is_none());
    }

    #[test]
    fn search_json_keyword_opens_independent_panel_not_provider() {
        use crate::plugin::manifest::{
            Compatibility, Contributions, PluginIdentity, PluginProvider, RuntimeSpec,
        };
        use crate::plugin::registry::{RegisteredPlugin, RuntimePhase};
        use crate::plugin::Trigger;

        fn devtools_plugin() -> RegisteredPlugin {
            RegisteredPlugin {
                manifest: crate::plugin::PluginManifest {
                    schema_version: 1,
                    plugin: PluginIdentity {
                        id: "com.kite.devtools".into(),
                        name: "开发者工具".into(),
                        version: "0.1.0".into(),
                        description: "uuid/hash/json".into(),
                        usage: String::new(),
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
                        providers: vec![PluginProvider {
                            id: "json".into(),
                            response_mode: "panel".into(),
                            triggers: vec![Trigger::Keyword { value: "json".into() }],
                        }],
                    },
                },
                root: std::path::PathBuf::from("plugins/com.kite.devtools"),
                enabled: true,
                phase: RuntimePhase::Dormant,
                last_error: None,
            }
        }

        let mut state = test_state("json");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(devtools_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged("json".into()));
        assert!(
            !state.plugin_tool_open && state.json_tool_window.is_none(),
            "搜索 json 不得直接打开工具窗，须二次确认"
        );
        assert!(state.provider_mode.is_none(), "不得进入 Provider");
        assert!(
            state.pending_tool_confirm.is_some(),
            "应进入二次确认"
        );
        assert!(state.results_stale == false);
        assert_eq!(state.results.len(), 1, "搜索 json 只展示 JSON 工具一条");
        assert_eq!(state.results[0].item.id, super::json_tool::CONFIRM_RESULT_ID);
        assert_eq!(state.results[0].item.name, "JSON 工具");
        // Enter = 打开
        let _ = launch_selected(&mut state);
        assert!(
            state.plugin_tool_open && state.json_tool_window.is_some(),
            "确认后才打开独立工具窗"
        );
        assert!(state.pending_tool_confirm.is_none());

        // Esc 取消路径
        let mut state = test_state("json");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(devtools_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged("json".into()));
        assert!(state.pending_tool_confirm.is_some());
        let _ = on_key(
            &mut state,
            Key::Named(Named::Escape),
            Physical::Code(Code::Escape),
            Modifiers::empty(),
        );
        assert!(state.pending_tool_confirm.is_none());
        assert!(!state.plugin_tool_open, "取消不得打开工具窗");

        let mut state = test_state("");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(devtools_plugin());
        }
        let _ = update(
            &mut state,
            Message::QueryChanged(r#"json {"a":1}"#.into()),
        );
        assert!(
            !state.plugin_tool_open,
            "带 payload 也不得直接开窗"
        );
        assert!(state.pending_tool_confirm.is_some());
        assert_eq!(
            state.pending_tool_confirm.as_ref().and_then(|p| p.payload.as_deref()),
            Some(r#"{"a":1}"#)
        );
        let _ = update(&mut state, Message::PluginConfirmJsonTool);
        assert!(state.plugin_tool_open);
        assert!(state.json_result.contains("\"a\": 1"), "确认后预填并格式化");
    }

    #[test]
    fn json_tool_opens_as_independent_panel_not_settings_subpage() {
        let mut state = test_state("");
        state.settings_open = true;
        state.settings_section = Section::Plugins;
        // 设置页：第一次只进入确认，不直接开窗
        let _ = update(&mut state, Message::PluginOpenJsonTool);
        assert!(
            !state.plugin_tool_open && state.json_tool_window.is_none(),
            "设置页打开面板也须二次确认"
        );
        assert!(state.pending_tool_confirm.as_ref().is_some_and(|p| p.from_settings));
        assert!(state.settings_open, "确认态不改主窗设置页");

        let _ = update(&mut state, Message::PluginConfirmJsonTool);
        assert!(state.plugin_tool_open);
        assert!(state.json_tool_window.is_some(), "工具应是独立 OS 窗口");
        assert!(
            state.settings_open,
            "打开 JSON 工具不得改动主启动器窗口（设置页保持原样）"
        );
        assert_eq!(state.settings_section, Section::Plugins);
        assert!(state.plugin_docs_open.is_none());

        state.json_editor =
            iced::widget::text_editor::Content::with_text(r#"{"a":1,"b":[true,null]}"#);
        let _ = update(&mut state, Message::JsonToolFormat);
        assert!(state.json_result.contains("\"a\": 1"));

        let _ = update(&mut state, Message::JsonToolMinify);
        assert!(!state.json_result.contains('\n'));

        let _ = update(&mut state, Message::PluginCloseJsonTool);
        assert!(!state.plugin_tool_open);
        assert!(state.json_tool_window.is_none());
        assert!(
            state.settings_open,
            "关闭工具窗后主窗设置页仍应保持打开"
        );
    }

    #[test]
    fn settings_json_tool_confirm_can_cancel_without_opening() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenJsonTool);
        assert!(state.pending_tool_confirm.is_some());
        let _ = update(&mut state, Message::PluginCancelJsonToolConfirm);
        assert!(state.pending_tool_confirm.is_none());
        assert!(!state.plugin_tool_open);
        assert!(state.settings_open);
    }

    #[test]
    fn blur_keeps_json_tool_panel_open() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenJsonTool);
        let _ = update(&mut state, Message::PluginConfirmJsonTool);
        let _ = update(&mut state, Message::WindowBlur);
        assert!(
            state.plugin_tool_open && state.json_tool_window.is_some(),
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
        let _ = update(&mut state, Message::PluginOpenJsonTool);
        let _ = update(&mut state, Message::PluginConfirmJsonTool);
        assert!(state.plugin_tool_open);
        let _ = update(&mut state, Message::PluginToggleDocs("com.kite.calculator".into()));
        assert!(
            state.plugin_tool_open && state.json_tool_window.is_some(),
            "打开说明不得关闭独立工具窗"
        );
        assert!(state.settings_open);
        assert_eq!(state.plugin_docs_open.as_deref(), Some("com.kite.calculator"));
    }

    #[test]
    fn json_tool_close_does_not_resize_or_switch_main_window() {
        let mut state = test_state("");
        state.settings_open = true;
        let _ = update(&mut state, Message::PluginOpenJsonTool);
        let _ = update(&mut state, Message::PluginConfirmJsonTool);
        assert!(state.json_tool_window.is_some());
        let _ = update(&mut state, Message::PluginCloseJsonTool);
        // 主窗状态保持：设置仍开、未隐藏、视图仍是设置而非工具。
        assert!(state.settings_open);
        assert!(!state.hidden);
        assert!(state.json_tool_window.is_none());
        assert!(!state.plugin_tool_open);
    }

    #[test]
    fn inline_calculator_provider_does_not_need_tool_confirm() {
        let mut state = test_state("=1+2");
        {
            let mut reg = state.plugin_registry.lock().unwrap();
            reg.insert_loaded(calculator_plugin());
        }
        let _ = update(&mut state, Message::QueryChanged("=1+2".into()));
        assert!(
            state.provider_mode.as_ref().is_some_and(|a| a.provider_id == "calculate"),
            "内联插件直接进 Provider，无需二次确认"
        );
        assert!(state.pending_tool_confirm.is_none());
        assert!(!state.plugin_tool_open);
    }
}

/// 采用后台 Full 挂起快照；Launcher 可见期间不调用。
pub(super) fn apply_pending_full(state: &mut State) {
    let Some(full) = super::backend::take_pending_full() else {
        return;
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
        return;
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
    state.index_generation = state.index_generation.wrapping_add(1);
    state.base_hit_cache.clear();
    state.refresh_results();
}

fn capture_menu_item(results: &[SearchResult], index: usize) -> Option<AppItem> {
    results.get(index).map(|result| result.item.clone())
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

        results.swap(0, 1);

        assert_eq!(clicked.id, "first");
        assert_eq!(clicked.target, r"C:\Apps\first.exe");
    }

    #[test]
    fn second_right_click_closes_context_menu() {
        use super::super::test_support::test_state;
        let mut state = test_state("");
        let _ = update(&mut state, Message::ContextMenu(0));
        assert!(state.menu.is_some(), "first right-click should open menu");

        let _ = update(&mut state, Message::ContextMenu(0));
        assert!(
            state.menu.is_none(),
            "second right-click should close menu"
        );
    }

    #[test]
    fn right_click_outside_does_not_close_before_context_menu() {
        use super::super::test_support::test_state;
        let mut state = test_state("");
        state.cursor.set(iced::Point::new(20.0, 30.0));
        let _ = update(&mut state, Message::ContextMenu(0));
        assert!(state.menu.is_some());

        // 右键不应走「点菜单外关闭」，否则 ContextMenu 会立刻重新打开
        let _ = update(&mut state, Message::MousePressed(iced::mouse::Button::Right));
        assert!(
            state.menu.is_some(),
            "right-press must not dismiss menu before ContextMenu toggle"
        );

        let _ = update(&mut state, Message::ContextMenu(0));
        assert!(state.menu.is_none());
    }

    #[test]
    fn left_click_outside_closes_context_menu() {
        use super::super::test_support::test_state;
        let mut state = test_state("");
        state.cursor.set(iced::Point::new(20.0, 30.0));
        let _ = update(&mut state, Message::ContextMenu(0));
        assert!(state.menu.is_some());

        // 菜单外左键应关闭
        state.cursor.set(iced::Point::new(600.0, 400.0));
        let _ = update(&mut state, Message::MousePressed(iced::mouse::Button::Left));
        assert!(state.menu.is_none(), "left click outside should close");
    }
}

fn persist_portable_dirs(state: &mut State) {
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

fn request_rescan(state: &State) {
    let tx = EVENT_TX.get().expect("event tx").clone();
    backend::request_build(
        state.index.clone(),
        state.icon_dir.clone(),
        state.scan_options.clone(),
        tx,
    );
}
