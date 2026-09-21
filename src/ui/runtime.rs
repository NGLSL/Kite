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

/// 窗口标题：工具窗按种类命名，主窗「Kite」。
fn poc_title(state: &State, window: window::Id) -> String {
    match state.tools.kind_of_window(window) {
        Some(kind) => kind.title().to_string(),
        None => "Kite".to_string(),
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

    let (index, index_ready) = warm_start_index();
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

    let tx = setup_event_bridge();
    spawn_hotkey_worker(saved_settings.hotkey.clone());
    spawn_index_builders(index.clone(), icon_dir.clone(), scan_options.clone(), tx);
    init_resources();
    spawn_tray();
    spawn_activation_listener();

    let mut state = build_state(data_dir, icon_dir, history_db, index, scan_options, index_ready);
    spawn_idle_sweep(&state.plugin_host);
    apply_saved_settings(&mut state, saved_settings);
    state.refresh_results();
    open_main_window(state)
}

fn warm_start_index() -> (std::sync::Arc<Mutex<AppIndex>>, bool) {
    let index = std::sync::Arc::new(Mutex::new(AppIndex::empty()));
    if let Some(loaded) = app::snapshot::load() {
        let n = loaded.apps.len();
        *index.lock().unwrap_or_else(|error| error.into_inner()) = loaded;
        plog(&format!("boot restored last-good snapshot n={n}"));
        return (index, true);
    }
    (index, false)
}

fn setup_event_bridge() -> iced::futures::channel::mpsc::UnboundedSender<Message> {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded::<Message>();
    let _ = EVENT_TX.set(tx.clone());
    let _ = EVENT_RX.set(Mutex::new(Some(rx)));
    tx
}

fn spawn_hotkey_worker(saved_hotkey: String) {
    use super::HotkeyCmd;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
        MOD_SHIFT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_HOTKEY,
    };
    const ID_MAIN: i32 = 0xB00B;
    const ID_REC_BASE: i32 = 0xB0C0;
    /// 录制期吞键：Windows 会把 Alt+Space 交给系统菜单，必须先 RegisterHotKey 抢下。
    const RECORD_SWALLOWS: &[(&str, u32, u32)] = &[
        ("Alt+Space", MOD_ALT.0 | MOD_NOREPEAT.0, 0x20),
        (
            "Ctrl+Alt+Space",
            MOD_ALT.0 | MOD_CONTROL.0 | MOD_NOREPEAT.0,
            0x20,
        ),
        (
            "Alt+Shift+Space",
            MOD_ALT.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0,
            0x20,
        ),
    ];
    let default_mods = (MOD_ALT | MOD_NOREPEAT).0;
    let (saved_mods, saved_vk) = parse_raw(&saved_hotkey).unwrap_or((default_mods, 0x20u32));
    let mut current = (HOT_KEY_MODIFIERS(saved_mods), saved_vk);
    let (hk_tx, hk_rx) = std::sync::mpsc::channel::<HotkeyCmd>();
    let _ = HOTKEY_CMD.set(hk_tx);
    std::thread::spawn(move || unsafe {
        let mut registered = match RegisterHotKey(None, ID_MAIN, current.0, current.1) {
            Ok(()) => {
                plog(&format!("hotkey registered {saved_hotkey}"));
                true
            }
            Err(error) => {
                plog(&format!(
                    "hotkey register failed for {saved_hotkey}; hotkey disabled; error={error}"
                ));
                send_event(Message::HotkeyUnavailable(saved_hotkey.clone()));
                false
            }
        };
        let mut recording = false;
        let mut swallow_ids: Vec<(i32, &'static str)> = Vec::new();

        fn end_record(
            recording: &mut bool,
            swallow_ids: &mut Vec<(i32, &'static str)>,
            registered: &mut bool,
            current: (HOT_KEY_MODIFIERS, u32),
        ) {
            if !*recording && swallow_ids.is_empty() {
                return;
            }
            *recording = false;
            for (id, _) in swallow_ids.drain(..) {
                unsafe {
                    let _ = UnregisterHotKey(None, id);
                }
            }
            if !*registered {
                *registered = unsafe { RegisterHotKey(None, ID_MAIN, current.0, current.1) }.is_ok();
                if *registered {
                    plog("hotkey restored after record");
                }
            }
        }

        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_HOTKEY {
                    let id = msg.wParam.0 as i32;
                    if let Some((_, spec)) = swallow_ids.iter().find(|(sid, _)| *sid == id) {
                        plog(&format!("hotkey record captured {spec}"));
                        send_event(Message::HotkeyRecordCaptured((*spec).to_string()));
                    } else if id == ID_MAIN && !recording {
                        plog("WM_HOTKEY received");
                        send_event(Message::Hotkey(Instant::now()));
                    }
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            while let Ok(cmd) = hk_rx.try_recv() {
                match cmd {
                    HotkeyCmd::BeginRecord => {
                        if registered {
                            let _ = UnregisterHotKey(None, ID_MAIN);
                            registered = false;
                        }
                        recording = true;
                        swallow_ids.clear();
                        for (offset, (spec, mods, vk)) in RECORD_SWALLOWS.iter().enumerate() {
                            let id = ID_REC_BASE + offset as i32;
                            match RegisterHotKey(None, id, HOT_KEY_MODIFIERS(*mods), *vk) {
                                Ok(()) => {
                                    swallow_ids.push((id, spec));
                                    plog(&format!("hotkey record swallow {spec}"));
                                }
                                Err(error) => {
                                    plog(&format!(
                                        "hotkey record swallow failed for {spec}; error={error}"
                                    ));
                                }
                            }
                        }
                    }
                    HotkeyCmd::EndRecord => {
                        end_record(
                            &mut recording,
                            &mut swallow_ids,
                            &mut registered,
                            current,
                        );
                    }
                    HotkeyCmd::Apply(spec) => {
                        if let Some((mods, vk)) = parse_raw(&spec) {
                            end_record(
                                &mut recording,
                                &mut swallow_ids,
                                &mut registered,
                                current,
                            );
                            let old_registered = registered;
                            if registered {
                                let _ = UnregisterHotKey(None, ID_MAIN);
                                registered = false;
                            }
                            let result =
                                match RegisterHotKey(None, ID_MAIN, HOT_KEY_MODIFIERS(mods), vk) {
                                    Ok(()) => {
                                        current = (HOT_KEY_MODIFIERS(mods), vk);
                                        registered = true;
                                        plog(&format!("hotkey re-registered: {spec}"));
                                        Ok(())
                                    }
                                    Err(error) => {
                                        if old_registered {
                                            registered = RegisterHotKey(
                                                None,
                                                ID_MAIN,
                                                current.0,
                                                current.1,
                                            )
                                            .is_ok();
                                        }
                                        plog(&format!(
                                            "hotkey register failed for {spec}; old_restored={registered}; error={error}"
                                        ));
                                        Err(error.to_string())
                                    }
                                };
                            send_event(Message::HotkeyRegistrationResult(spec, result));
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
}

fn spawn_index_builders(
    index: std::sync::Arc<Mutex<AppIndex>>,
    icon_dir: PathBuf,
    scan_options: std::sync::Arc<std::sync::RwLock<app::scanner::ScanOptions>>,
    tx: iced::futures::channel::mpsc::UnboundedSender<Message>,
) {
    backend::request_build(index.clone(), icon_dir.clone(), scan_options.clone(), tx.clone());
    let w_index = index;
    let w_dir = icon_dir;
    let w_tx = tx;
    app::watch::spawn_entry_watchers(scan_options.clone(), move || {
        backend::request_build(
            w_index.clone(),
            w_dir.clone(),
            scan_options.clone(),
            w_tx.clone(),
        );
    });
}

fn init_resources() {
    if let Some(rd) = resource_dir() {
        system::resources::init(rd);
        plog("resources initialized (Everything64.dll located)");
    } else {
        plog("resources dir not found; file search disabled");
    }
}

fn spawn_tray() {
    if let Some(tx) = event_tx() {
        tray::spawn(tx, include_bytes!("../../icons/32x32.png"));
    }
}

fn spawn_activation_listener() {
    if let Err(error) = system::singleton::spawn_activation_listener(|| {
        send_event(Message::EnsureVisible);
    }) {
        plog(&format!("activation listener not started: {error}"));
    }
}

fn spawn_idle_sweep(plugin_host: &std::sync::Arc<Mutex<PluginHost>>) {
    let host = std::sync::Arc::downgrade(plugin_host);
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

fn apply_saved_settings(state: &mut State, saved_settings: storage::settings::Settings) {
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
}

fn build_state(
    data_dir: PathBuf,
    icon_dir: PathBuf,
    history_db: Option<HistoryDb>,
    index: std::sync::Arc<Mutex<AppIndex>>,
    scan_options: std::sync::Arc<std::sync::RwLock<app::scanner::ScanOptions>>,
    index_ready: bool,
) -> State {
    let saved_settings = history_db
        .as_ref()
        .map(HistoryDb::load_settings)
        .unwrap_or_default();
    State {
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
        direct_path_generation: 0,
        direct_path_latest: Default::default(),
        direct_path_results: Vec::new(),
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
        plugin_query_worker: std::sync::Arc::new(plugin::PluginQueryWorker::spawn()),
        plugin_flash: None,
        plugin_import_path: String::new(),
        plugin_docs_open: None,
        tools: Default::default(),
        pending_tool_confirm: None,
        prefs_cache: None,
    }
}

fn open_main_window(mut state: State) -> (State, Task<Message>) {
    let launcher_settings = Settings {
        size: iced::Size::new(WINDOW_W, WINDOW_H),
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
        Subscription::run(events_worker),
        keyboard::keyboard_events(state),
        window::close_events().map(Message::ToolWindowClosed),
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
