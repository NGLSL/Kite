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
                Some(id) if state.hidden => {
                    state.hidden = false;
                    state.epoch += 1;
                    state.refresh_results();
                    // 对齐 Kite：唤起即响（SND_ASYNC，不阻塞显示）
                    system::sound::play_open();
                    plog(&format!("show issued epoch={}", state.epoch));
                    Task::batch([
                        // 搜索窗口只需要前台焦点，不应持续置顶，否则会压住截图层和其它全局快捷键 UI。
                        window::set_level(id, window::Level::Normal),
                        window::set_mode(id, window::Mode::Windowed),
                        window::gain_focus(id),
                        iced::widget::operation::focus(state.input_id.clone()),
                        sync_scroll(state),
                    ])
                }
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
        Message::KeyPressed(key, physical, mods) => {
            // 本地跟踪 Alt（SYSKEY 下 modifiers.alt() 可能为假）
            if matches!(key, Key::Named(Named::Alt)) {
                state.alt_down = true;
                state.query_at_alt = Some(state.query.clone());
                state.alt_digit_consumed = false;
                plog("alt down");
            }
            on_key(state, key, physical, mods)
        }
        Message::KeyReleased(key, _mods) => {
            if matches!(key, Key::Named(Named::Alt)) {
                state.alt_down = false;
                state.query_at_alt = None;
                state.alt_digit_consumed = false;
                plog("alt up");
            }
            Task::none()
        }
        Message::Composing(active) => {
            if state.ime_composing != active {
                plog(&format!("ime composing={active}"));
                state.ime_composing = active;
            }
            Task::none()
        }
        Message::ImeCommit(text) => {
            plog(&format!("ime commit '{text}'"));
            Task::none()
        }
        Message::WindowBlur => {
            // 对齐 Kite：设置页打开时失焦不隐藏
            if state.settings_open {
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
            state.window_id = id;
            if state.settings_open {
                id.map(settings_window_task).unwrap_or_else(Task::none)
            } else {
                Task::none()
            }
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
                    plog(&format!("alt-digit text fallback idx={i}"));
                    return launch_alt_digit(state, i);
                }
            }
            state.query = q;
            state.refresh_results();
            sync_scroll(state)
        }
        Message::ClearQuery => {
            state.query.clear();
            state.refresh_results();
            // × 按钮会抢走键盘焦点，清空后还给输入框
            Task::batch([
                iced::widget::operation::focus(state.input_id.clone()),
                sync_scroll(state),
            ])
        }
        Message::HoverSelect(i) => {
            // 对齐前端 onMouseMove：悬停只改选中，不滚动（否则滚轮一滚就被 scroll_to 拽走）
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
            plog(&format!("files toggle -> {}", state.files_mode));
            state.refresh_results();
            Task::none()
        }
        Message::Rescan => {
            plog("rescan requested from tray");
            let index = state.index.clone();
            let dir = state.icon_dir.clone();
            let options = state.scan_options.clone();
            let tx = EVENT_TX.get().expect("event tx").clone();
            backend::request_build(index, dir, options, tx);
            Task::none()
        }
        Message::Quit => {
            plog("quit requested");
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
            plog(&format!("open releases err={r:?}"));
            if r.is_err() {
                flash(state, "打开 GitHub 最新发布页失败")
            } else {
                Task::none()
            }
        }
        Message::OpenRepository => {
            let r = app::uwp::launch_shell_path(system::update::REPOSITORY_URL);
            plog(&format!("open repository err={r:?}"));
            if r.is_err() {
                flash(state, "打开 Kite 仓库失败")
            } else {
                Task::none()
            }
        }
        Message::HotkeyUnavailable(spec) => {
            plog(&format!("hotkey unavailable: {spec}"));
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
                plog(&format!("hotkey change rejected: {spec}; error={error}"));
                flash(state, &format!("快捷键 {spec} 无法使用，请换一个组合键"))
            }
        },
        Message::OpenSettings => open_settings(state),
        Message::CloseSettings => close_settings(state),
        Message::SettingsSection(s) => {
            state.settings_section = s;
            Task::none()
        }
        Message::SetAutostart(v) => {
            state.autostart = v;
            if let Some(db) = &mut state.history {
                let _ = db.save_setting("autostart", if v { "1" } else { "0" });
            }
            let r = system::autostart::set_autostart(v);
            plog(&format!("autostart -> {v} err={r:?}"));
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
            flash(state, "别名已保存")
        }
        Message::AliasRemove(alias) => {
            if let Some(db) = &mut state.history {
                let _ = db.remove_alias(&alias);
            }
            load_aliases(state);
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
            Task::none()
        }
        Message::MousePressed(_btn) => {
            // 前端行为：菜单开着时点击菜单外任意位置 → 关闭（点在菜单内交给菜单项按钮）
            if let Some((idx, x, y)) = state.menu {
                let c = state.cursor.get();
                let target = state
                    .results
                    .get(idx)
                    .map(|r| r.item.target.clone())
                    .unwrap_or_default();
                let n = if std::path::Path::new(&target).is_file() {
                    4
                } else {
                    2
                };
                let h = 8.0 + n as f32 * 33.0;
                if c.x < x || c.x > x + 180.0 || c.y < y || c.y > y + h {
                    state.menu = None;
                }
            }
            Task::none()
        }
        Message::ContextMenu(i) => {
            let c = state.cursor.get();
            let x = c.x.min(640.0 - 200.0).max(8.0);
            let y = c.y.min(420.0 - 180.0).max(8.0);
            state.menu = Some((i, x, y));
            Task::none()
        }
        Message::MenuAction(i, action) => menu_action(state, i, action),
        Message::DragWindow => state.window_id.map(window::drag).unwrap_or_else(Task::none),
        Message::IndexReady(n) => {
            plog(&format!("index ready n={n}"));
            state.index_ready = true;
            state.refresh_results();
            Task::none()
        }
        Message::IconsFilled(n) => {
            plog(&format!("icons filled n={n}"));
            state.refresh_results();
            Task::none()
        }
        Message::UwpMerged(n) => {
            plog(&format!("uwp merged n={n}"));
            state.refresh_results();
            Task::none()
        }
        Message::FullIndexReady(n) => {
            plog(&format!("full index ready n={n}"));
            state.index_ready = true;
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
            state.refresh_results();
            Task::none()
        }
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
