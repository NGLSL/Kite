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
                    state.qlog(|| format!("alt-digit text fallback idx={i}"));
                    return launch_alt_digit(state, i);
                }
            }
            state.query = q;
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
