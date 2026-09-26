//! 窗口生命周期、热键、托盘、索引就绪与更新检查。

use super::actions::{flash, hide, hide_task, on_key, open_settings, settings_window_task, show_launcher};
use super::interaction::apply_pending_full;
use super::{plog, send_event, system, Message, State};
use crate::app;
use iced::{window, Task};

pub(super) fn handle(state: &mut State, message: Message) -> Task<Message> {
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
            if matches!(
                key,
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Alt)
            ) {
                state.alt_down = true;
                state.query_at_alt = Some(state.query.clone());
                state.alt_digit_consumed = false;
                state.qlog(|| "alt down".to_string());
            }
            on_key(state, key, physical, mods)
        }
        Message::KeyReleased(key, _mods) => {
            if matches!(
                key,
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Alt)
            ) {
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
            if state.settings_open || state.any_tool_open() {
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
            if state.settings_open {
                id.map(settings_window_task).unwrap_or_else(Task::none)
            } else {
                Task::none()
            }
        }
        Message::DragWindow => state
            .window_id
            .map(window::drag)
            .unwrap_or_else(Task::none),
        Message::Quit => {
            state.qlog(|| "quit requested".to_owned());
            std::process::exit(0);
        }
        Message::FlashClear => {
            state.flash = None;
            Task::none()
        }
        Message::FullIndexReady(n) => {
            plog(&format!("full index ready n={n}"));
            state.index_ready = true;
            // 活跃输入要立即看到新入口；结果导航中保留当前列表，等下一次查询或隐藏时采用。
            let active_query = !state.query.trim().is_empty()
                && state.navigation_mode == super::NavigationMode::Input;
            if (state.hidden || state.rescan_pending || active_query) && apply_pending_full(state) {
                state.refresh_results();
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
            state.index_generation = state.index_generation.wrapping_add(1);
            state.base_hit_cache.clear();
            state.invalidate_prefs_cache();
            state.refresh_results();
            Task::none()
        }
        Message::HotkeyUnavailable(spec) => {
            state.qlog(|| format!("hotkey unavailable: {spec}"));
            let show = open_settings(state);
            state.settings_section = super::Section::Hotkey;
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
        Message::CheckUpdate => {
            if state.update_checking {
                return Task::none();
            }
            state.update_checking = true;
            state.update_status = None;
            state.update_asset = None;
            std::thread::spawn(|| {
                let result = system::update::check_latest(env!("CARGO_PKG_VERSION"));
                send_event(Message::UpdateResult(result));
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
                    send_event(Message::UpdateInstallerLaunched);
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
                send_event(Message::UpdateResult(Err(error)));
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
        _ => Task::none(),
    }
}
