//! Iced 启动、后台索引与消息桥。

use super::*;

/// 原生 UI 启动入口（lib::run 调用）。
pub fn run() -> iced::Result {
    // 数据/缓存目录与旧版 Kite 完全一致（com.kite.launcher）
    let data_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.kite.launcher");
    let icon_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("com.kite.launcher")
        .join("icons");
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::create_dir_all(&icon_dir);
    let _ = ICON_DIR.set(icon_dir.clone());
    log::init(data_dir.join("kite.log"));
    plog(&format!(
        "start version={} pid={} data_dir={:?} icon_dir={:?}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        data_dir,
        icon_dir
    ));

    let _ = BOOT_DIR.set(data_dir.clone());
    app::snapshot::init(data_dir.clone());
    app::index_health::init(data_dir.clone());
    // Daemon 多窗口：boot 里 window::open 主启动器；JSON 工具为独立第二窗。
    iced::daemon(boot_entry, update, view)
        .title(poc_title)
        .theme(poc_theme)
        .default_font(font::ui_font())
        .subscription(subscription)
        .run()
}

/// 窗口标题：工具窗「JSON 工具」，主窗「Kite」。
fn poc_title(state: &State, window: window::Id) -> String {
    if state.json_tool_window == Some(window) {
        "JSON 工具".to_string()
    } else if state.hash_tool_window == Some(window) {
        "Hash 工具".to_string()
    } else {
        "Kite".to_string()
    }
}

fn poc_theme(state: &State, _window: window::Id) -> Option<Theme> {
    if state.theme_mode.is_dark() {
        Some(Theme::Dark)
    } else {
        Some(Theme::Light)
    }
}

/// 定位 resources（含 Everything64.dll / open.wav）。
fn resource_dir() -> Option<PathBuf> {
    if let Ok(env) = std::env::var("KITE_POC_RESOURCES") {
        let p = PathBuf::from(env);
        if p.join("Everything64.dll").exists() || p.join("open.wav").exists() {
            return Some(p);
        }
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [
        exe_dir.join("resources"),
        exe_dir.join("../resources"),
        exe_dir.join("../../resources"),
        exe_dir.join("../../../resources"),
    ]
    .into_iter()
    .find(|p| p.join("Everything64.dll").exists() || p.join("open.wav").exists())
}

fn boot_entry() -> (State, Task<Message>) {
    let data_dir = BOOT_DIR
        .get()
        .cloned()
        .expect("boot dir set before application run");
    let icon_dir = ICON_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| data_dir.join("icons"));
    boot(data_dir, icon_dir)
}

fn boot(data_dir: PathBuf, icon_dir: PathBuf) -> (State, Task<Message>) {
    let history_db = HistoryDb::open(&data_dir.join("kite-history.db"))
        .map_err(|e| plog(&format!("history db open failed: {e}")))
        .ok();

    let index = std::sync::Arc::new(Mutex::new(AppIndex::empty()));
    // Warm Start：兼容 last-good 直接恢复可搜索快照；失败则保持 empty，走 Cold Bootstrap。
    let mut index_ready = false;
    if let Some(loaded) = app::snapshot::load() {
        let n = loaded.apps.len();
        *index.lock().unwrap_or_else(|error| error.into_inner()) = loaded;
        // 已有可搜索 RetrievalIndex：语义上应视为 ready，不必等 FullIndexReady。
        index_ready = true;
        plog(&format!("boot restored last-good snapshot n={n}"));
    }
    let saved_settings = history_db
        .as_ref()
        .map(HistoryDb::load_settings)
        .unwrap_or_default();
    let scan_options = std::sync::Arc::new(std::sync::RwLock::new(app::scanner::ScanOptions {
        portable_dirs: saved_settings
            .portable_dirs
            .iter()
            .map(PathBuf::from)
            .collect(),
        ..app::scanner::ScanOptions::default()
    }));

    let (tx, rx) = iced::futures::channel::mpsc::unbounded::<Message>();
    let _ = EVENT_TX.set(tx.clone());
    let _ = EVENT_RX.set(Mutex::new(Some(rx)));

    // 先读取持久化快捷键，再启动注册线程。否则升级/重启后线程会先注册
    // 默认 Alt+Space，而 UI 随后才加载用户配置，保存的快捷键永远不会生效。
    let saved_hotkey = saved_settings.hotkey.clone();

    // 快捷键线程：原生 RegisterHotKey（线程关联）+ 消息泵 + 改键命令轮询。
    // 注意：必须在本线程泵消息（GetMessageW），WM_HOTKEY 才会被投递；注册失败
    // 只禁用热键并保留托盘，用户仍可从设置页改键。
    let (hk_tx, hk_rx) = std::sync::mpsc::channel::<String>();
    let _ = HOTKEY_CMD.set(hk_tx);
    std::thread::spawn(move || {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_NOREPEAT,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_HOTKEY,
        };
        const ID: i32 = 0xB00B;
        let default_mods = (MOD_ALT | MOD_NOREPEAT).0;
        let (saved_mods, saved_vk) = parse_raw(&saved_hotkey).unwrap_or((default_mods, 0x20u32));
        let mut current = (HOT_KEY_MODIFIERS(saved_mods), saved_vk);
        unsafe {
            let mut registered = match RegisterHotKey(None, ID, current.0, current.1) {
                Ok(()) => {
                    plog(&format!("hotkey registered {saved_hotkey}"));
                    true
                }
                Err(error) => {
                    plog(&format!(
                        "hotkey register failed for {saved_hotkey}; hotkey disabled; error={error}"
                    ));
                    // Keep the worker alive so settings can register a new key.
                    let _ = EVENT_TX
                        .get()
                        .expect("event tx")
                        .unbounded_send(Message::HotkeyUnavailable(saved_hotkey.clone()));
                    false
                }
            };
            let mut msg = MSG::default();
            loop {
                // 泵全部待处理消息
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_HOTKEY {
                        plog("WM_HOTKEY received");
                        let _ = EVENT_TX
                            .get()
                            .expect("event tx")
                            .unbounded_send(Message::Hotkey(Instant::now()));
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                // 设置页改键：注销旧键 → 注册新键（失败回退旧键）。
                // 初始键位被占用时 registered=false，仍可在此处改键。
                if let Ok(spec) = hk_rx.try_recv() {
                    if let Some((mods, vk)) = parse_raw(&spec) {
                        let old_registered = registered;
                        if registered {
                            let _ = UnregisterHotKey(None, ID);
                            registered = false;
                        }
                        let result = match RegisterHotKey(None, ID, HOT_KEY_MODIFIERS(mods), vk) {
                            Ok(()) => {
                                current = (HOT_KEY_MODIFIERS(mods), vk);
                                registered = true;
                                plog(&format!("hotkey re-registered: {spec}"));
                                Ok(())
                            }
                            Err(error) => {
                                if old_registered {
                                    registered =
                                        RegisterHotKey(None, ID, current.0, current.1).is_ok();
                                }
                                plog(&format!(
                                    "hotkey register failed for {spec}; old_restored={registered}; error={error}"
                                ));
                                Err(error.to_string())
                            }
                        };
                        let _ = EVENT_TX
                            .get()
                            .expect("event tx")
                            .unbounded_send(Message::HotkeyRegistrationResult(spec, result));
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    });

    // 索引线程：后台构建完整快照，完成后一次发布（单飞）
    {
        let index = index.clone();
        let dir = icon_dir.clone();
        let options = scan_options.clone();
        backend::request_build(index.clone(), dir.clone(), options.clone(), tx.clone());
        // 入口变化监听：debounce 合并后触发重建
        let w_index = index;
        let w_dir = dir;
        let w_tx = tx.clone();
        app::watch::spawn_entry_watchers(options.clone(), move || {
            backend::request_build(
                w_index.clone(),
                w_dir.clone(),
                options.clone(),
                w_tx.clone(),
            );
        });
    }

    // 资源目录（Everything64.dll）：优先 exe 旁，落到仓库 resources
    if let Some(rd) = resource_dir() {
        system::resources::init(rd);
        plog("resources initialized (Everything64.dll located)");
    } else {
        plog("resources dir not found; file search disabled");
    }

    // 托盘常驻：独立线程，左键/菜单事件走 events 桥
    tray::spawn(
        EVENT_TX.get().expect("event tx").clone(),
        include_bytes!("../../icons/32x32.png"),
    );

    // 二次启动激活：主实例已在 lib::run claim；此处消息桥就绪后开始监听。
    if let Err(error) = system::singleton::spawn_activation_listener(|| {
        if let Some(tx) = EVENT_TX.get() {
            let _ = tx.unbounded_send(Message::EnsureVisible);
        }
    }) {
        plog(&format!("activation listener not started: {error}"));
    }

    let mut state = State {
        data_dir: data_dir.clone(),
        icon_dir: icon_dir.clone(),
        index,
        scan_options,
        history: history_db,
        input_id: search_view::input_id(),
        window_id: None,
        query: String::new(),
        results: Vec::new(),
        selected: 0,
        navigation_mode: NavigationMode::Input,
        hidden: true,
        ime_composing: false,
        alt_down: false,
        query_at_alt: None,
        alt_digit_consumed: false,
        index_ready,
        rescan_pending: false,
        files_mode: false,
        file_filter: system::everything::FileFilter::All,
        file_query_generation: 0,
        file_results: Vec::new(),
        app_query_generation: 0,
        results_stale: false,
        index_generation: 0,
        base_hit_cache: std::sync::Arc::new(search::service::BaseHitCache::default()),
        app_search_worker: std::sync::Arc::new(search::service::AppSearchWorker::spawn()),
        menu: None,
        pinned: Default::default(),
        cursor: Default::default(),
        settings_open: false,
        settings_section: Section::General,
        hide_on_blur: true,
        autostart: false,
        history_recording: true,
        theme_mode: ThemeMode::parse(&saved_settings.theme_mode),
        grid_recent_count: 0,
        query_log: true,
        search_engine: "auto".into(),
        search_engine_custom: String::new(),
        web_search_hotkey: system::hotkey::DEFAULT_WEB_SEARCH_HOTKEY.into(),
        hotkey: "Alt+Space".into(),
        hotkey_label: "Alt+Space".into(),
        aliases: Vec::new(),
        alias_input: String::new(),
        alias_target_input: String::new(),
        alias_candidates: Vec::new(),
        alias_pick: None,
        portable_dirs: saved_settings.portable_dirs.clone(),
        portable_dir_input: String::new(),
        flash: None,
        hotkey_recording: false,
        update_status: None,
        update_asset: None,
        update_checking: false,
        epoch: 0,
        hover_suppressed: false,
        last_hover_pt: None,
        plugin_registry: std::sync::Arc::new(Mutex::new({
            let plugins_dir = plugin::default_plugins_dir(&data_dir);
            // 官方插件随安装包提供；每次启动同步到用户目录，覆盖安装包旧版本。
            let _ = plugin::sync_official_plugins(&plugins_dir);
            plugin::load_registry_from_dir(&plugins_dir)
        })),
        plugin_host: std::sync::Arc::new(Mutex::new(PluginHost::new(
            Box::new(plugin::process::StdioBackend::new(data_dir.clone())),
            data_dir.clone(),
            env!("CARGO_PKG_VERSION"),
        ))),
        provider_mode: None,
        plugin_panel: None,
        plugin_query_generation: 0,
        plugin_flash: None,
        plugin_import_path: String::new(),
        plugin_docs_open: None,
        json_tool_window: None,
        hash_tool_window: None,
        plugin_tool_open: false,
        pending_tool_confirm: None,
        json_editor: iced::widget::text_editor::Content::default(),
        json_result: String::new(),
        json_tool_note: None,
        hash_editor: iced::widget::text_editor::Content::default(),
        hash_result: String::new(),
        hash_tool_note: None,
    };
    // Idle Shutdown：定时清扫，不依赖下一次 Provider 触发。
    {
        // 后台清扫线程不能持有强引用，否则 State 退出后 PluginHost 永远不 Drop，
        // 子插件进程会变成孤儿并锁住下一次安装要覆盖的文件。
        let host = std::sync::Arc::downgrade(&state.plugin_host);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let Some(host) = host.upgrade() else {
                break;
            };
            {
                let guard = host.lock();
                if let Ok(mut h) = guard {
                    h.idle_sweep();
                }
            }
        });
    }
    // 启动时载入设置（副本库）
    state.hide_on_blur = saved_settings.hide_on_blur;
    state.autostart = saved_settings.autostart;
    state.history_recording = saved_settings.history_recording;
    state.query_log = saved_settings.query_log;
    state.search_engine = saved_settings.search_engine.clone();
    state.web_search_hotkey = saved_settings.web_search_hotkey.clone();
    state.search_engine_custom = state
        .history
        .as_ref()
        .and_then(|db| db.search_url_template())
        .or_else(|| {
            system::search_engine::preset_by_id(&state.search_engine)
                .filter(|p| !p.template.is_empty())
                .map(|p| p.template.to_string())
        })
        .unwrap_or_default();
    state.hotkey = saved_settings.hotkey;
    state.hotkey_label = saved_settings.hotkey_label;
    state.refresh_results();
    // 主启动器窗口：hidden 启动；JSON 工具窗口在触发时另行 window::open。
    let launcher_settings = Settings {
        size: iced::Size::new(WINDOW_W, WINDOW_H),
        // 对齐 Kite：水平居中、y = 屏高 1/3（system/window.rs place_on_current_monitor）
        position: Position::SpecificWith(|win, monitor| {
            iced::Point::new((monitor.width - win.width) / 2.0, monitor.height / 3.0)
        }),
        visible: false,
        resizable: false,
        decorations: false,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        platform_specific: PlatformSpecific {
            skip_taskbar: true,
            // 无边框窗口保留系统投影（对齐 tauri shadow:true，白底不至于融入桌面）
            undecorated_shadow: false,
            corner_preference: iced::window::settings::platform::CornerPreference::Round,
            ..PlatformSpecific::default()
        },
        ..Settings::default()
    };
    let (main_id, open_main) = window::open(launcher_settings);
    state.window_id = Some(main_id);
    (state, open_main.map(|id| Message::WindowReady(Some(id))))
}

fn subscription(state: &State) -> Subscription<Message> {
    Subscription::batch([
        // 后台线程消息桥（快捷键、索引构建完成等）
        Subscription::run(events_worker),
        // 键盘 + IME + 窗口焦点事件
        keyboard::keyboard_events(state),
        // 工具窗被系统关闭时清理 json_tool_window
        window::close_events().map(Message::JsonToolWindowClosed),
    ])
}

/// 后台线程消息桥：先取走 receiver（一次性），再真异步收消息。
fn events_worker() -> impl iced::futures::Stream<Item = Message> {
    use iced::futures::SinkExt;
    use iced::futures::StreamExt;
    stream::channel(64, async move |mut sender| {
        plog("events worker started");
        let mut rx = EVENT_RX
            .get()
            .expect("events rx initialized in boot")
            .lock()
            .expect("events rx lock")
            .take()
            .expect("events rx taken once");
        while let Some(msg) = rx.next().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    })
}
