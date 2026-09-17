//! 搜索窗口的选择、启动、设置和快捷键动作。

use super::*;

/// 上下文菜单动作。
pub(super) fn menu_action(state: &mut State, item: AppItem, action: MenuAction) -> Task<Message> {
    state.menu = None;
    match action {
        MenuAction::OpenFolder => {
            let r = app::actions::open_containing_folder(&item.target);
            state.qlog(|| format!("ctx open_folder err={r:?}"));
        }
        MenuAction::CopyPath => return iced::clipboard::write(item.target),
        MenuAction::CopyTarget => {
            let t = item.target.trim();
            let body = t
                .strip_prefix("shell:AppsFolder\\")
                .or_else(|| t.strip_prefix("shell:appsfolder\\"))
                .unwrap_or(t);
            return iced::clipboard::write(body.to_string());
        }
        MenuAction::CopyName => return iced::clipboard::write(item.display_name),
        MenuAction::TogglePin => {
            if let Some(db) = &mut state.history {
                let r = if state.pinned.contains(&item.id) {
                    db.unpin_item(&item.id)
                } else {
                    db.pin_item(&item.id, storage::now_ts())
                };
                state.qlog(|| format!("ctx pin toggle ok={}", r.is_ok()));
            }
            // 与 Demote 共用同一入口：非空 Query 同样交给常驻 worker 重排。
            state.refresh_results();
        }
        MenuAction::Demote | MenuAction::Undemote => {
            if let Some(db) = &mut state.history {
                let r = if matches!(action, MenuAction::Demote) {
                    db.demote_item(&item.id, storage::now_ts())
                } else {
                    db.undemote_item(&item.id)
                };
                state.qlog(|| {
                    format!(
                        "ctx demote action={action:?} ok={} id={}",
                        r.is_ok(),
                        item.id
                    )
                });
            }
            state.refresh_results();
        }
    }
    Task::none()
}

/// 键盘：↑↓ 选择、Enter 启动（组合态禁止，见 ime_composing）、Esc 关菜单/隐藏、
/// Alt+1..9 启动对应行（对齐前端 index<9 提示）；热键录制态优先捕获。
pub(super) fn on_key(
    state: &mut State,
    key: Key,
    physical: Physical,
    mods: Modifiers,
) -> Task<Message> {
    if state.hidden {
        return Task::none();
    }
    if state.hotkey_recording {
        return hotkey_record_key(state, key, mods);
    }

    // Alt+1..9：兼容 modifiers.alt() / 本地 alt_down / 逻辑字符 / 物理 Digit|Numpad
    let alt_idx = alt_digit_index(&key, physical, mods, state.alt_down);
    if let Some(i) = alt_idx {
        let alt_down = state.alt_down;
        let mods_alt = mods.alt();
        state.qlog(|| {
            format!("alt-n idx={i} alt_down={alt_down} mods_alt={mods_alt} key={key:?} phys={physical:?}")
        });
        return launch_alt_digit(state, i);
    }

    match key {
        Key::Named(Named::Escape) if !state.ime_composing => {
            // 前端行为：菜单开着时 Esc 只关菜单；设置页开着时 Esc 回搜索
            if state.menu.take().is_some() {
                state.qlog(|| "ctx menu closed (esc)".to_owned());
                return Task::none();
            }
            if state.settings_open {
                return close_settings(state);
            }
            state.qlog(|| "hide issued (esc)".to_owned());
            hide(state);
            hide_task(state)
        }
        Key::Named(Named::ArrowUp) => move_selection(state, -1),
        Key::Named(Named::ArrowDown) => move_selection(state, 1),
        Key::Named(Named::Enter) if !state.ime_composing => {
            if web_search_hotkey_pressed(state, mods) {
                launch_browser_search(state)
            } else {
                launch_selected(state)
            }
        }
        Key::Named(Named::Alt) => Task::none(),
        Key::Named(name) => {
            state.qlog(|| format!("key named {name:?} ignored"));
            Task::none()
        }
        Key::Character(c) => {
            // 普通输入交给 text_input；这里只记日志（避免与 QueryChanged 双处理）
            let _ = c;
            Task::none()
        }
        _ => Task::none(),
    }
}

/// 窗口内网页搜索快捷键是否按下（默认 Ctrl+Enter；设置可改）。
fn web_search_hotkey_pressed(state: &State, mods: Modifiers) -> bool {
    let spec = system::hotkey::normalize_web_search_hotkey(&state.web_search_hotkey);
    match spec {
        "Ctrl+Enter" => {
            mods.control() && !mods.alt() && !mods.shift() && !mods.logo()
        }
        "Alt+Enter" => mods.alt() && !mods.control() && !mods.shift() && !mods.logo(),
        "Shift+Enter" => {
            mods.shift() && !mods.control() && !mods.alt() && !mods.logo()
        }
        "Ctrl+Shift+Enter" => {
            mods.control() && mods.shift() && !mods.alt() && !mods.logo()
        }
        _ => false,
    }
}

/// 用当前 Query 直接打开浏览器搜索（不依赖列表选中项）。
fn launch_browser_search(state: &mut State) -> Task<Message> {
    let q = state.query.trim().to_string();
    if q.is_empty() || state.settings_open || state.hidden {
        return Task::none();
    }
    let preferred = state.history.as_ref().and_then(|h| h.preferred_browser());
    let template = state.history.as_ref().and_then(|h| h.search_url_template());
    let browser_id = preferred.as_deref().unwrap_or("default");
    let t0 = Instant::now();
    system::env::refresh_process_env();
    let result = app::web::launch_websearch(browser_id, &q, template.as_deref());
    match result {
        Ok(preferred_id) => {
            state.qlog(|| {
                format!(
                    "websearch hotkey via {browser_id} in {}us",
                    t0.elapsed().as_micros()
                )
            });
            if let Some(db) = &mut state.history {
                let qn = search::normalize_for_index(&q);
                let id = app::web::make_search_id(browser_id, &q);
                let _ = db.record_launch(&id, &qn, storage::now_ts());
                if let Some(pid) = preferred_id {
                    let _ = db.set_preferred_browser(&pid);
                }
            }
            hide(state);
            hide_task(state)
        }
        Err(e) => {
            state.qlog(|| format!("websearch hotkey failed: {e}"));
            Task::none()
        }
    }
}

pub(super) fn launch_alt_digit(state: &mut State, i: usize) -> Task<Message> {
    if state.ime_composing
        || state.settings_open
        || state.alt_digit_consumed
        || i >= state.results.len()
    {
        return Task::none();
    }
    state.alt_digit_consumed = true;
    // KeyPressed 可能晚于 text_input，保留按下 Alt 前的搜索文本。
    if let Some(before) = state.query_at_alt.clone() {
        if state.query != before {
            state.query = before;
            state.refresh_results();
            // 列表仍是按下 Alt 前那次搜索的结果——用户按 Alt+N 要的就是它。
            // 上面的重新提交只是为了文本与请求一致，不该让本次启动被判为过期。
            state.results_stale = false;
        }
    }
    state.selected = i;
    launch_selected(state)
}

/// 选中行滚入可视区（保留上方两行），对应前端滚动定位行为。
pub(super) fn sync_scroll(state: &State) -> Task<Message> {
    let y =
        ((state.selected as f32) * search_view::ROW_STEP - 2.0 * search_view::ROW_STEP).max(0.0);
    scroll_to(search_view::scroll_id(), AbsoluteOffset { x: 0.0, y })
}

pub(super) fn move_selection(state: &mut State, delta: i32) -> Task<Message> {
    if state.results.is_empty() {
        return Task::none();
    }
    let len = state.results.len() as i32;
    let next = (state.selected as i32 + delta).clamp(0, len - 1);
    state.selected = next as usize;
    // scroll_to 会让鼠标底下换成另一行并触发 on_enter；先抑制悬停改选。
    state.hover_suppressed = true;
    sync_scroll(state)
}

pub(super) fn launch_selected(state: &mut State) -> Task<Message> {
    // 启动动作只对「当前查询的结果」生效。新查询提交后旧列表仍在屏幕上
    // （避免闪空），但此时启动它就会打开上一次搜索的应用。Enter / Alt+数字 /
    // 鼠标点击都汇到这里，有效性判定因此只有一处。
    if !state.results_are_launchable() {
        state.qlog(|| "launch ignored: results belong to a previous query".to_owned());
        return Task::none();
    }
    let Some(item) = state.results.get(state.selected).map(|r| r.item.clone()) else {
        state.qlog(|| "enter with empty results; ignored".to_owned());
        return Task::none();
    };
    state.menu = None;

    // 内置：打开 Kite 设置
    if item.id == "kite:settings" {
        return open_settings(state);
    }

    if let Some(url) = everything_download_url(&item.id) {
        let result = app::uwp::launch_shell_path(url);
        state.qlog(|| format!("open Everything download err={result:?}"));
        return if result.is_ok() {
            hide(state);
            hide_task(state)
        } else {
            flash(state, "无法打开 Everything 官方下载页")
        };
    }
    if item.id == system::everything::NOT_RUNNING_RESULT_ID {
        return flash(state, "请先启动 Everything，再使用文件搜索");
    }

    // 浏览器打开网址 / 网页搜索（id 由 app::web 生成，对齐 commands::launch_app）
    if let Some((kind, browser_id, payload)) = app::web::parse_id(&item.id) {
        let t0 = Instant::now();
        system::env::refresh_process_env();
        let cached = state.history.as_ref().and_then(|h| h.search_url_template());
        let result = if kind == "websearch" {
            app::web::launch_websearch(&browser_id, &payload, cached.as_deref())
        } else {
            app::web::launch_url(&browser_id, &payload)
        };
        return match result {
            Ok(preferred) => {
                let elapsed_us = t0.elapsed().as_micros();
                state.qlog(|| format!("launch web {kind} via {browser_id} in {elapsed_us}us"));
                if let Some(db) = &mut state.history {
                    let q = search::normalize_for_index(&state.query);
                    let _ = db.record_launch(&item.id, &q, storage::now_ts());
                    if let Some(pid) = preferred {
                        let _ = db.set_preferred_browser(&pid);
                    }
                }
                state.qlog(|| "hide issued (launch)".to_owned());
                hide(state);
                hide_task(state)
            }
            Err(e) => {
                state.qlog(|| format!("launch web {kind} failed: {e}"));
                Task::none()
            }
        };
    }

    let t0 = Instant::now();
    // 对齐 commands::launch_app：先刷新进程环境再拉起
    system::env::refresh_process_env();
    match app::launch(&item) {
        Ok(()) => {
            let elapsed_us = t0.elapsed().as_micros();
            let (name, target) = (item.display_name.clone(), item.target.clone());
            state.qlog(|| format!("launch '{name}' target={target} in {elapsed_us}us ok"));
            if let Some(db) = &mut state.history {
                let q = search::normalize_for_index(&state.query);
                let _ = db.record_launch(&item.id, &q, storage::now_ts());
            }
            state.qlog(|| "hide issued (launch)".to_owned());
            hide(state);
            hide_task(state)
        }
        Err(e) => {
            state.qlog(|| {
                format!(
                    "launch '{}' target={} failed: {e}",
                    item.display_name, item.target
                )
            });
            Task::none()
        }
    }
}

fn everything_download_url(item_id: &str) -> Option<&'static str> {
    (item_id == system::everything::DOWNLOAD_RESULT_ID).then_some(system::everything::DOWNLOAD_URL)
}

#[cfg(test)]
mod everything_action_tests {
    use super::*;

    #[test]
    fn missing_dependency_result_targets_official_download_page() {
        assert_eq!(
            everything_download_url(system::everything::DOWNLOAD_RESULT_ID),
            Some("https://www.voidtools.com/downloads/")
        );
        assert_eq!(
            everything_download_url(system::everything::NOT_RUNNING_RESULT_ID),
            None
        );
    }
}

#[cfg(test)]
mod interactive_log_gate_tests {
    use super::super::test_support::test_state;
    use super::*;
    use crate::log;
    use crate::ui::interaction::update;
    use iced::keyboard::key::Code;

    fn press(state: &mut State, key: Key) -> Task<Message> {
        on_key(state, key, Physical::Code(Code::KeyA), Modifiers::empty())
    }

    /// 关掉查询日志后，交互路径（Esc、未识别具名键、Alt+数字、启动、右键菜单入口、
    /// 文件模式切换、设置页开合）不得产生任何日志行 —— 不调用 `plog`，
    /// 也就没有同步文件写入。
    #[test]
    fn interactive_path_writes_no_log_when_query_log_off() {
        let mut state = test_state("k");
        state.query_log = false;

        log::test_records();
        // Esc：隐藏启动器
        let _ = press(&mut state, Key::Named(Named::Escape));
        // 未识别的具名键
        let _ = press(&mut state, Key::Named(Named::Tab));
        // 启动：结果过期时被拒
        state.hidden = false;
        state.results_stale = true;
        let _ = launch_selected(&mut state);
        // 启动：内置「Kite 设置」→ 设置页打开，再关掉
        state.results_stale = false;
        let _ = launch_selected(&mut state);
        let _ = close_settings(&mut state);
        // Alt+数字
        state.hidden = false;
        state.alt_digit_consumed = false;
        let _ = on_key(
            &mut state,
            Key::Character("1".into()),
            Physical::Code(Code::Digit1),
            Modifiers::ALT,
        );
        // 文件模式切换与文件搜索落地／过期
        let _ = update(&mut state, Message::ToggleFiles);
        let _ = update(
            &mut state,
            Message::FileSearchReady(0, String::new(), Vec::new(), 0),
        );
        // 设置窗口内的动作：快捷键变更被拒
        let _ = update(
            &mut state,
            Message::HotkeyRegistrationResult("Alt+X".into(), Err("boom".into())),
        );

        assert_eq!(
            log::test_records(),
            Vec::<String>::new(),
            "关闭查询日志后，交互路径不得产生任何同步日志写入"
        );
    }

    /// 开关开启（默认）时，同样的交互路径照常写日志 —— 证明上面的断言确实覆盖了这些点。
    #[test]
    fn interactive_path_still_logs_when_query_log_on() {
        let mut state = test_state("k");
        assert!(state.query_log, "查询日志默认开启");

        log::test_records();
        let _ = press(&mut state, Key::Named(Named::Escape));
        let _ = update(&mut state, Message::ToggleFiles);
        let _ = update(
            &mut state,
            Message::FileSearchReady(0, String::new(), Vec::new(), 0),
        );
        let _ = update(
            &mut state,
            Message::HotkeyRegistrationResult("Alt+X".into(), Err("boom".into())),
        );

        let records = log::test_records();
        assert!(
            records.iter().any(|r| r.contains("hide issued (esc)")),
            "开启时 Esc 必须仍写日志，实际：{records:?}"
        );
        assert!(
            records.iter().any(|r| r.contains("files toggle ->")),
            "开启时文件模式切换必须仍写日志，实际：{records:?}"
        );
        assert!(
            records.iter().any(|r| r.contains("file search ")),
            "开启时文件搜索落地必须仍写日志，实际：{records:?}"
        );
        assert!(
            records.iter().any(|r| r.contains("hotkey change rejected")),
            "开启时设置窗口动作必须仍写日志，实际：{records:?}"
        );
    }
}

/// 把逻辑尺寸窗口摆到光标所在显示器工作区中部（物理定位 → 按窗口当前 scale 转逻辑）。
fn place_on_cursor_monitor_task(
    id: window::Id,
    window_w: f32,
    window_h: f32,
) -> Task<Message> {
    window::scale_factor(id).then(move |scale| {
        let Some((px, py)) =
            system::window_place::physical_position_on_cursor_monitor(window_w, window_h)
        else {
            return Task::none();
        };
        let (lx, ly) = system::window_place::physical_to_logical_for_window((px, py), scale);
        window::move_to(id, iced::Point::new(lx, ly))
    })
}

/// 显示主窗口（热键/托盘/二次启动共用）。窗口未就绪时忽略。
pub(super) fn show_launcher(state: &mut State) -> Task<Message> {
    let Some(id) = state.window_id else {
        plog("show before window ready; ignored");
        return Task::none();
    };
    // 下次打开时采用挂起的 Full，避免可见期间打断列表。
    super::interaction::apply_pending_full(state);
    state.hidden = false;
    state.epoch += 1;
    // 每次唤起按鼠标所在 monitor 重新定位（uTools 式多显示器跟随）。
    let place = system::window_place::physical_position_on_cursor_monitor(WINDOW_W, WINDOW_H);
    if state.files_mode {
        state.request_file_search();
    }
    state.refresh_results();
    // 对齐 Kite：唤起即响（SND_ASYNC，不阻塞显示）
    system::sound::play_open();
    plog(&format!(
        "show issued epoch={} place={place:?}",
        state.epoch
    ));
    Task::batch([
        // 搜索窗口只需要前台焦点，不应持续置顶，否则会压住截图层和其它全局快捷键 UI。
        window::set_level(id, window::Level::Normal),
        // gain_focus 在窗口不可见时是 no-op：必须先 set_mode 再 focus，不能 batch 并行。
        window::set_mode(id, window::Mode::Windowed)
            .chain(window::gain_focus(id))
            .chain(iced::widget::operation::focus(state.input_id.clone())),
        sync_scroll(state),
        place_on_cursor_monitor_task(id, WINDOW_W, WINDOW_H),
    ])
}

pub(super) fn hide(state: &mut State) {
    state.hidden = true;
    state.ime_composing = false;
    state.alt_down = false;
    state.query_at_alt = None;
    state.alt_digit_consumed = false;
    state.menu = None;
    state.query.clear();
    state.invalidate_file_search();
    // 隐藏后采用后台 Full，用户下次看到的就是新快照。
    super::interaction::apply_pending_full(state);
    state.refresh_results();
}

pub(super) fn hide_task(state: &State) -> Task<Message> {
    state
        .window_id
        .map(|id| window::set_mode(id, window::Mode::Hidden))
        .unwrap_or_else(Task::none)
}

/// 设置页提示条（css settings-toast），2s 自动消失。
pub(super) fn flash(state: &mut State, msg: &str) -> Task<Message> {
    state.flash = Some(msg.to_string());
    Task::perform(
        async {
            std::thread::sleep(std::time::Duration::from_secs(2));
        },
        |()| Message::FlashClear,
    )
}

/// 打开设置：载入设置与别名，窗口切到 720×520（对齐 set_settings_mode）。
pub(super) fn open_settings(state: &mut State) -> Task<Message> {
    state.settings_open = true;
    state.hidden = false;
    state.settings_section = Section::General;
    state.menu = None;
    if let Some(db) = &state.history {
        let s = db.load_settings();
        state.hide_on_blur = s.hide_on_blur;
        state.autostart = s.autostart;
        state.history_recording = s.history_recording;
        state.query_log = s.query_log;
        state.search_engine = s.search_engine.clone();
        state.web_search_hotkey = s.web_search_hotkey.clone();
        state.search_engine_custom = db
            .search_url_template()
            .or_else(|| {
                system::search_engine::preset_by_id(&state.search_engine)
                    .filter(|p| !p.template.is_empty())
                    .map(|p| p.template.to_string())
            })
            .unwrap_or_default();
        state.hotkey = s.hotkey.clone();
        state.hotkey_label = s.hotkey_label.clone();
        state.portable_dirs = s.portable_dirs;
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
    load_aliases(state);
    state.qlog(|| "settings open".to_owned());
    let Some(id) = state.window_id else {
        return Task::none();
    };
    Task::batch([
        settings_window_task(id),
        // 与 show_launcher 一致：物理定位 + 窗口 scale 转逻辑，避免 2K/1K 缩放偏移。
        place_on_cursor_monitor_task(id, SETTINGS_W, SETTINGS_H),
    ])
}

const SETTINGS_W: f32 = 720.0;
const SETTINGS_H: f32 = 520.0;

pub(super) fn settings_window_task(id: window::Id) -> Task<Message> {
    // gain_focus 在窗口不可见时是 no-op：必须先 set_mode/resize 再 focus，不能 batch 并行。
    window::set_level(id, window::Level::Normal)
        .chain(window::set_mode(id, window::Mode::Windowed))
        .chain(window::resize(
            id,
            iced::Size::new(SETTINGS_W, SETTINGS_H),
        ))
        .chain(window::gain_focus(id))
}

/// 关闭设置：窗口切回搜索尺寸并聚焦输入框。
pub(super) fn close_settings(state: &mut State) -> Task<Message> {
    state.settings_open = false;
    state.flash = None;
    state.qlog(|| "settings close".to_owned());
    Task::batch([
        state
            .window_id
            .map(|id| window::set_level(id, window::Level::Normal))
            .unwrap_or_else(Task::none),
        state
            .window_id
            .map(|id| window::resize(id, iced::Size::new(WINDOW_W, WINDOW_H)))
            .unwrap_or_else(Task::none),
        iced::widget::operation::focus(state.input_id.clone()),
    ])
}

pub(super) fn load_aliases(state: &mut State) {
    if let Some(db) = &state.history {
        if let Ok(list) = db.list_aliases() {
            state.aliases = list;
        }
    }
}

/// 别名增删改后的统一失效入口。
///
/// 别名目标直接决定搜索里明确匹配保护层的命中，因此基础候选缓存以及正在跑的
/// 请求都要作废：否则同一个输入会继续按旧别名排序；而只清空缓存也不够——
/// 在途任务完成时还会把用旧别名算出的候选写回来（由缓存代际拦住）。
pub(super) fn refresh_after_alias_change(state: &mut State) {
    state.base_hit_cache.invalidate();
    state.refresh_results();
}

/// 热键录制态：下一组按键即新快捷键（Esc 取消）。
pub(super) fn hotkey_record_key(state: &mut State, key: Key, mods: Modifiers) -> Task<Message> {
    if let Key::Named(Named::Escape) = key {
        state.hotkey_recording = false;
        return flash(state, "已取消");
    }
    let key_name: Option<String> = match &key {
        Key::Character(c) => c
            .chars()
            .next()
            .map(|ch| ch.to_ascii_uppercase().to_string()),
        Key::Named(n) => match n {
            Named::Space => Some("Space".into()),
            Named::Tab => Some("Tab".into()),
            Named::F1 => Some("F1".into()),
            Named::F2 => Some("F2".into()),
            Named::F3 => Some("F3".into()),
            Named::F4 => Some("F4".into()),
            Named::F5 => Some("F5".into()),
            Named::F6 => Some("F6".into()),
            Named::F7 => Some("F7".into()),
            Named::F8 => Some("F8".into()),
            Named::F9 => Some("F9".into()),
            Named::F10 => Some("F10".into()),
            Named::F11 => Some("F11".into()),
            Named::F12 => Some("F12".into()),
            _ => None,
        },
        _ => None,
    };
    let Some(k) = key_name else {
        return Task::none();
    };
    let mut parts: Vec<&str> = Vec::new();
    if mods.control() {
        parts.push("Ctrl");
    }
    if mods.alt() {
        parts.push("Alt");
    }
    if mods.shift() {
        parts.push("Shift");
    }
    if mods.logo() {
        parts.push("Win");
    }
    if parts.is_empty() {
        return flash(state, "请连同修饰键一起按下，例如 Ctrl+Alt+K");
    }
    let spec = {
        let mut s = parts.join("+");
        s.push('+');
        s.push_str(&k);
        s
    };
    state.hotkey_recording = false;
    Task::done(Message::ApplyHotkey(spec))
}
