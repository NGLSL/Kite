//! 原生主界面（iced + tiny-skia 软渲染，无 WebView2）：窗口、托盘、全局热键、
//! 搜索/设置/别名/固定/文件/网页搜索的全部交互层。业务逻辑复用本 crate 各模块。
//!
//! 日志：%APPDATA%\com.kite.launcher\kite.log；数据目录与旧版 Kite 一致，
//! 历史库/设置直接读写真实数据。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use iced::keyboard::{key::Named, Key, Modifiers};
use iced::widget::operation::scroll_to;
use iced::widget::scrollable::AbsoluteOffset;
use iced::widget::Id as WidgetId;
use iced::window::settings::PlatformSpecific;
use iced::window::{self, Position, Settings};
use iced::{keyboard, stream, Subscription, Task, Theme};

use crate::model::{AppIndex, AppItem, SearchResult};
use crate::storage::settings::UserAlias;
use crate::storage::HistoryDb;
use crate::{app, history, log, search, storage, system};
use crate::system::hotkey::parse_raw;

mod backend;
mod font;
mod settings_view;
mod search_view;
mod tray;

const WINDOW_W: f32 = 640.0;
const WINDOW_H: f32 = 420.0;

/// boot 里生成的跨线程消息通道：后台线程 → iced runtime。
/// 用 futures 的 unbounded channel：worker 侧 `next().await` 真异步等待
/// （waker 唤醒，空闲零 CPU；绝不能在 async 里用 std mpsc 的阻塞 recv——
/// 那会把 poll 它的执行器线程原地挂死，消息永远到不了 update）。
static EVENT_TX: OnceLock<iced::futures::channel::mpsc::UnboundedSender<Message>> = OnceLock::new();
static EVENT_RX: OnceLock<Mutex<Option<iced::futures::channel::mpsc::UnboundedReceiver<Message>>>> =
    OnceLock::new();
static BOOT_DIR: OnceLock<PathBuf> = OnceLock::new();
static ICON_DIR: OnceLock<PathBuf> = OnceLock::new();
/// 热键线程的改键命令通道（"Alt+Space" → 运行时重注册）。
static HOTKEY_CMD: OnceLock<std::sync::mpsc::Sender<String>> = OnceLock::new();

/// 设置页分区（对齐前端 NAV）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    General,
    Hotkey,
    Alias,
    Index,
    About,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::General,
        Section::Hotkey,
        Section::Alias,
        Section::Index,
        Section::About,
    ];
    fn label(self) -> &'static str {
        match self {
            Section::General => "通用",
            Section::Hotkey => "热键",
            Section::Alias => "别名",
            Section::Index => "应用索引",
            Section::About => "关于",
        }
    }
}

pub(crate) fn plog(msg: &str) {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    log::info(&format!("[{ms}] {msg}"));
}

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
    iced::application(boot_entry, update, view)
    .title("Kite")
    .window(Settings {
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
    })
    .theme(poc_theme)
    .default_font(font::ui_font())
    .subscription(subscription)
    .run()
}

/// BootFn 需要 `Fn`；fn 指针比闭包省去生命周期推断问题，目录经 BOOT_DIR 传递。
fn poc_theme(_state: &State) -> Theme {
    Theme::Light
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

#[derive(Debug, Clone)]
enum Message {
    /// 全局快捷键触发；携带接收线程的单调时间戳（埋点起点）。
    Hotkey(Instant),
    KeyPressed(Key, Modifiers),
    /// IME 组合态变化（true = 候选期间）。
    Composing(bool),
    /// IME 提交了文本（埋点用，插入由 text_input 自行处理）。
    ImeCommit(String),
    WindowBlur,
    WindowReady(Option<window::Id>),
    QueryChanged(String),
    ClearQuery,
    /// 鼠标悬停选中（css onMouseMove）。
    HoverSelect(usize),
    /// 鼠标点击 / Alt+N 启动第 i 行。
    LaunchIndex(usize),
    /// 「文件」胶囊开关（css .files-toggle）。
    ToggleFiles,
    /// 托盘菜单：重新扫描应用。
    Rescan,
    /// 托盘菜单：退出。
    Quit,
    /// 鼠标移动（右键菜单锚定用；写 Cell 不改变视图语义）。
    CursorMoved(iced::Point),
    /// 任意鼠标按下（菜单外点击关闭菜单）。
    MousePressed(iced::mouse::Button),
    /// 右键第 i 行：打开上下文菜单（css .ctx-menu）。
    ContextMenu(usize),
    /// 上下文菜单动作。
    MenuAction(usize, MenuAction),
    /// 按住拖拽窗口（css data-tauri-drag-region）。
    DragWindow,
    // ── 设置页 ──
    /// 打开设置（搜索结果 kite:settings / 托盘菜单）。
    OpenSettings,
    /// 关闭设置回到搜索。
    CloseSettings,
    /// 切换设置分区。
    SettingsSection(Section),
    SetAutostart(bool),
    SetHideOnBlur(bool),
    SetHistoryRecording(bool),
    /// 清空使用历史。
    ClearHistory,
    /// 应用新快捷键（预设 chips / 录制结果，"Alt+Space" 形式）。
    ApplyHotkey(String),
    AliasInputChanged(String),
    AliasTargetChanged(String),
    /// 选定第 i 个候选目标。
    AliasPick(usize),
    AliasAdd,
    AliasRemove(String),
    /// 设置页提示条自动消失。
    FlashClear,
    /// 热键「更改」按钮：进入录制态。
    StartHotkeyRecord,
    /// 检查 GitHub 更新（关于页）。
    CheckUpdate,
    DownloadUpdate,
    UpdateInstallerLaunched,
    /// GitHub Release 检查结果。
    UpdateResult(Result<system::update::CheckResult, String>),
    /// 打开发布页。
    OpenReleases,
    /// 打开 Kite 的 GitHub 仓库。
    OpenRepository,
    /// 全局快捷键被其他程序占用；应用仍可通过托盘打开并重新设置快捷键。
    HotkeyUnavailable(String),
    /// 快捷键线程确认本次改键是否真正注册成功。
    HotkeyRegistrationResult(String, Result<(), String>),
    IndexReady(usize),
    IconsFilled(usize),
    UwpMerged(usize),
    /// 后台完整扫描完成并已原子替换快照。
    FullIndexReady(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    OpenFolder,
    CopyPath,
    CopyName,
    TogglePin,
}

struct State {
    #[allow(dead_code)]
    data_dir: PathBuf,
    /// Everything64.dll 所在资源目录（文件搜索用）。
    icon_dir: PathBuf,
    index: std::sync::Arc<Mutex<AppIndex>>,
    history: Option<HistoryDb>,
    input_id: WidgetId,
    window_id: Option<window::Id>,
    query: String,
    results: Vec<SearchResult>,
    selected: usize,
    hidden: bool,
    ime_composing: bool,
    index_ready: bool,
    files_mode: bool,
    /// 右键菜单：(结果下标, x, y)。
    menu: Option<(usize, f32, f32)>,
    pinned: std::collections::HashSet<String>,
    /// 最新光标（窗口逻辑坐标；Cell 写入不参与视图比较）。
    cursor: std::rc::Rc<std::cell::Cell<iced::Point>>,
    // ── 设置页 ──
    settings_open: bool,
    settings_section: Section,
    hide_on_blur: bool,
    autostart: bool,
    history_recording: bool,
    hotkey: String,
    hotkey_label: String,
    aliases: Vec<UserAlias>,
    alias_input: String,
    alias_target_input: String,
    alias_candidates: Vec<SearchResult>,
    alias_pick: Option<UserAlias>,
    /// 设置页提示条（css settings-toast）。
    flash: Option<String>,
    /// 热键录制态（设置页「更改」）。
    hotkey_recording: bool,
    /// 更新检查：None=未检查；Some(Ok(latest))=完成；Some(Err(e))=失败。
    update_status: Option<Result<String, String>>,
    update_asset: Option<system::update::InstallerAsset>,
    update_checking: bool,
    /// 唤起轮次，埋点对齐用。
    epoch: u64,
}


fn boot(data_dir: PathBuf, icon_dir: PathBuf) -> (State, Task<Message>) {
    let history_db = HistoryDb::open(&data_dir.join("kite-history.db"))
        .map_err(|e| plog(&format!("history db open failed: {e}")))
        .ok();

    let index = std::sync::Arc::new(Mutex::new(AppIndex::empty()));

    let (tx, rx) = iced::futures::channel::mpsc::unbounded::<Message>();
    let _ = EVENT_TX.set(tx.clone());
    let _ = EVENT_RX.set(Mutex::new(Some(rx)));

    // 先读取持久化快捷键，再启动注册线程。否则升级/重启后线程会先注册
    // 默认 Alt+Space，而 UI 随后才加载用户配置，保存的快捷键永远不会生效。
    let saved_hotkey = history_db
        .as_ref()
        .map(|db| db.load_settings().hotkey)
        .unwrap_or_else(|| "Alt+Space".to_string());

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
            DispatchMessageW, MSG, PeekMessageW, PM_REMOVE, TranslateMessage,
            WM_HOTKEY,
        };
        const ID: i32 = 0xB00B;
        let default_mods = (MOD_ALT | MOD_NOREPEAT).0;
        let (saved_mods, saved_vk) = parse_raw(&saved_hotkey)
            .unwrap_or((default_mods, 0x20u32));
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
                                    registered = RegisterHotKey(None, ID, current.0, current.1).is_ok();
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

    // 索引线程：快扫首屏 + 图标/UWP + 后台完整补扫（单飞）
    {
        let index = index.clone();
        let dir = icon_dir.clone();
        backend::request_build(index, dir, tx.clone());
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

    let mut state = State {
        data_dir: data_dir.clone(),
        icon_dir: icon_dir.clone(),
        index,
        history: history_db,
        input_id: search_view::input_id(),
        window_id: None,
        query: String::new(),
        results: Vec::new(),
        selected: 0,
        hidden: true,
        ime_composing: false,
        index_ready: false,
        files_mode: false,
        menu: None,
        pinned: Default::default(),
        cursor: Default::default(),
        settings_open: false,
        settings_section: Section::General,
        hide_on_blur: true,
        autostart: false,
        history_recording: true,
        hotkey: "Alt+Space".into(),
        hotkey_label: "Alt+Space".into(),
        aliases: Vec::new(),
        alias_input: String::new(),
        alias_target_input: String::new(),
        alias_candidates: Vec::new(),
        alias_pick: None,
        flash: None,
        hotkey_recording: false,
        update_status: None,
        update_asset: None,
        update_checking: false,
        epoch: 0,
    };
    // 启动时载入设置（副本库）
    if let Some(db) = &state.history {
        let s = db.load_settings();
        state.hide_on_blur = s.hide_on_blur;
        state.autostart = s.autostart;
        state.history_recording = s.history_recording;
        state.hotkey = s.hotkey.clone();
        state.hotkey_label = s.hotkey_label.clone();
    }
    state.refresh_results();
    // 窗口 open 完成后 boot task 才执行，此时 latest() 拿到主窗口 id
    (state, window::latest().map(Message::WindowReady))
}

fn subscription(state: &State) -> Subscription<Message> {
    Subscription::batch([
        // 后台线程消息桥（快捷键、索引构建完成等）
        Subscription::run(events_worker),
        // 键盘 + IME + 窗口焦点事件
        keyboard_events(state),
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

fn keyboard_events(state: &State) -> Subscription<Message> {
    // 正常运行时只订阅 Kite 自己使用的按键；否则外部截图、录屏和辅助工具的
    // 单键快捷键会先进入 Iced 窗口。录制快捷键时才临时接收完整按键流。
    if state.hotkey_recording {
        iced::event::listen().filter_map(keyboard_message_recording)
    } else {
        iced::event::listen().filter_map(keyboard_message_app)
    }
}

fn keyboard_message_recording(event: iced::event::Event) -> Option<Message> {
    keyboard_message(event, true)
}

fn keyboard_message_app(event: iced::event::Event) -> Option<Message> {
    keyboard_message(event, false)
}

fn keyboard_message(event: iced::event::Event, recording: bool) -> Option<Message> {
    match event {
        iced::event::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
            if recording || app_key(&key, modifiers) =>
        {
            Some(Message::KeyPressed(key, modifiers))
        }
        iced::event::Event::InputMethod(im) => match im {
            iced_core::input_method::Event::Opened => Some(Message::Composing(true)),
            iced_core::input_method::Event::Preedit(s, _) => Some(Message::Composing(!s.is_empty())),
            iced_core::input_method::Event::Commit(s) => Some(Message::ImeCommit(s)),
            iced_core::input_method::Event::Closed => Some(Message::Composing(false)),
        },
        iced::event::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::CursorMoved(position))
        }
        iced::event::Event::Window(window::Event::Unfocused) => Some(Message::WindowBlur),
        iced::event::Event::Mouse(iced::mouse::Event::ButtonPressed(btn)) => {
            Some(Message::MousePressed(btn))
        }
        _ => None,
    }
}

fn app_key(key: &Key, mods: Modifiers) -> bool {
    match key {
        Key::Named(
            Named::Escape | Named::ArrowUp | Named::ArrowDown | Named::Enter,
        ) => true,
        Key::Character(c) => {
            mods.alt() && c.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        }
        _ => false,
    }
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Hotkey(t0) => {
            plog(&format!("hotkey recv->update {}us", t0.elapsed().as_micros()));
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
        Message::KeyPressed(key, mods) => on_key(state, key, mods),
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
            let tx = EVENT_TX.get().expect("event tx").clone();
            backend::request_build(index, dir, tx);
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
                plog(&format!("update automatic install failed: {:?}", result.err()));
                let fallback = app::uwp::launch_shell_path(system::update::LATEST_RELEASE_URL);
                plog(&format!("update fallback releases err={fallback:?}"));
                let error = if fallback.is_ok() {
                    "自动安装失败，已打开 GitHub 最新发布页，请手动安装".to_string()
                } else {
                    "自动安装失败，打开 GitHub 最新发布页也失败".to_string()
                };
                let _ = EVENT_TX.get().expect("event tx").unbounded_send(Message::UpdateResult(Err(error)));
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
                let n = if std::path::Path::new(&target).is_file() { 4 } else { 2 };
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
        Message::DragWindow => state
            .window_id
            .map(window::drag)
            .unwrap_or_else(Task::none),
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
            state.refresh_results();
            Task::none()
        }
    }
}

/// 上下文菜单动作（对齐 commands::result_action + 前端复制项）。
fn menu_action(state: &mut State, i: usize, action: MenuAction) -> Task<Message> {
    state.menu = None;
    let Some(item) = state.results.get(i).map(|r| r.item.clone()) else {
        return Task::none();
    };
    match action {
        MenuAction::OpenFolder => {
            let r = app::actions::open_containing_folder(&item.target);
            plog(&format!("ctx open_folder err={r:?}"));
        }
        MenuAction::CopyPath => return iced::clipboard::write(item.target),
        MenuAction::CopyName => return iced::clipboard::write(item.display_name),
        MenuAction::TogglePin => {
            if let Some(db) = &mut state.history {
                let r = if state.pinned.contains(&item.id) {
                    db.unpin_item(&item.id)
                } else {
                    db.pin_item(&item.id, storage::now_ts())
                };
                plog(&format!("ctx pin toggle ok={}", r.is_ok()));
            }
            state.refresh_results();
        }
    }
    Task::none()
}

/// 键盘：↑↓ 选择、Enter 启动（组合态禁止，见 ime_composing）、Esc 关菜单/隐藏、
/// Alt+1..9 启动对应行（对齐前端 index<9 提示）；热键录制态优先捕获。
fn on_key(state: &mut State, key: Key, mods: Modifiers) -> Task<Message> {
    if state.hotkey_recording {
        return hotkey_record_key(state, key, mods);
    }
    match key {
        Key::Named(Named::Escape) if !state.ime_composing => {
            // 前端行为：菜单开着时 Esc 只关菜单；设置页开着时 Esc 回搜索
            if state.menu.take().is_some() {
                plog("ctx menu closed (esc)");
                return Task::none();
            }
            if state.settings_open {
                return close_settings(state);
            }
            plog("hide issued (esc)");
            hide(state);
            hide_task(state)
        }
        Key::Named(Named::ArrowUp) => move_selection(state, -1),
        Key::Named(Named::ArrowDown) => move_selection(state, 1),
        Key::Named(Named::Enter) if !state.ime_composing => launch_selected(state),
        Key::Named(name) => {
            plog(&format!("key named {name:?} ignored"));
            Task::none()
        }
        Key::Character(c) => {
            if mods.alt() {
                if let Some(digit) = c.chars().next().and_then(|ch| ch.to_digit(10)) {
                    if digit >= 1 && !state.ime_composing {
                        return Task::done(Message::LaunchIndex((digit - 1) as usize));
                    }
                }
            }
            plog(&format!("key char '{c}'"));
            Task::none()
        }
        _ => Task::none(),
    }
}

/// 选中行滚入可视区（保留上方两行），对应前端滚动定位行为。
fn sync_scroll(state: &State) -> Task<Message> {
    let y = ((state.selected as f32) * search_view::ROW_STEP - 2.0 * search_view::ROW_STEP).max(0.0);
    scroll_to(search_view::scroll_id(), AbsoluteOffset { x: 0.0, y })
}

fn move_selection(state: &mut State, delta: i32) -> Task<Message> {
    if state.results.is_empty() {
        return Task::none();
    }
    let len = state.results.len() as i32;
    let next = (state.selected as i32 + delta).clamp(0, len - 1);
    state.selected = next as usize;
    sync_scroll(state)
}

fn launch_selected(state: &mut State) -> Task<Message> {
    let Some(item) = state.results.get(state.selected).map(|r| r.item.clone()) else {
        plog("enter with empty results; ignored");
        return Task::none();
    };
    state.menu = None;

    // 内置：打开 Kite 设置
    if item.id == "kite:settings" {
        return open_settings(state);
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
                plog(&format!(
                    "launch web {kind} via {browser_id} in {}us",
                    t0.elapsed().as_micros()
                ));
                if let Some(db) = &mut state.history {
                    let q = search::normalize_for_index(&state.query);
                    let _ = db.record_launch(&item.id, &q, storage::now_ts());
                    if let Some(pid) = preferred {
                        let _ = db.set_preferred_browser(&pid);
                    }
                }
                plog("hide issued (launch)");
                hide(state);
                hide_task(state)
            }
            Err(e) => {
                plog(&format!("launch web {kind} failed: {e}"));
                Task::none()
            }
        };
    }

    let t0 = Instant::now();
    // 对齐 commands::launch_app：先刷新进程环境再拉起
    system::env::refresh_process_env();
    match app::launch(&item) {
        Ok(()) => {
            plog(&format!(
                "launch '{}' target={} in {}us ok",
                item.display_name,
                item.target,
                t0.elapsed().as_micros()
            ));
            if let Some(db) = &mut state.history {
                let q = search::normalize_for_index(&state.query);
                let _ = db.record_launch(&item.id, &q, storage::now_ts());
            }
            plog("hide issued (launch)");
            hide(state);
            hide_task(state)
        }
        Err(e) => {
            plog(&format!(
                "launch '{}' target={} failed: {e}",
                item.display_name, item.target
            ));
            Task::none()
        }
    }
}

fn hide(state: &mut State) {
    state.hidden = true;
    state.ime_composing = false;
    state.menu = None;
    state.query.clear();
    state.refresh_results();
}

fn hide_task(state: &State) -> Task<Message> {
    state
        .window_id
        .map(|id| window::set_mode(id, window::Mode::Hidden))
        .unwrap_or_else(Task::none)
}

/// 设置页提示条（css settings-toast），2s 自动消失。
fn flash(state: &mut State, msg: &str) -> Task<Message> {
    state.flash = Some(msg.to_string());
    Task::perform(
        async {
            std::thread::sleep(std::time::Duration::from_secs(2));
        },
        |()| Message::FlashClear,
    )
}

/// 打开设置：载入设置与别名，窗口切到 720×520（对齐 set_settings_mode）。
fn open_settings(state: &mut State) -> Task<Message> {
    state.settings_open = true;
    state.hidden = false;
    state.settings_section = Section::General;
    state.menu = None;
    if let Some(db) = &state.history {
        let s = db.load_settings();
        state.hide_on_blur = s.hide_on_blur;
        state.autostart = s.autostart;
        state.history_recording = s.history_recording;
        state.hotkey = s.hotkey.clone();
        state.hotkey_label = s.hotkey_label.clone();
    }
    load_aliases(state);
    plog("settings open");
    state.window_id
        .map(settings_window_task)
        .unwrap_or_else(Task::none)
}

fn settings_window_task(id: window::Id) -> Task<Message> {
    Task::batch([
        window::set_level(id, window::Level::Normal),
        window::set_mode(id, window::Mode::Windowed),
        window::resize(id, iced::Size::new(720.0, 520.0)),
        window::gain_focus(id),
    ])
}

/// 关闭设置：窗口切回搜索尺寸并聚焦输入框。
fn close_settings(state: &mut State) -> Task<Message> {
    state.settings_open = false;
    state.flash = None;
    plog("settings close");
    Task::batch([
        state.window_id.map(|id| window::set_level(id, window::Level::Normal)).unwrap_or_else(Task::none),
        state
            .window_id
            .map(|id| window::resize(id, iced::Size::new(WINDOW_W, WINDOW_H)))
            .unwrap_or_else(Task::none),
        iced::widget::operation::focus(state.input_id.clone()),
    ])
}

fn load_aliases(state: &mut State) {
    if let Some(db) = &state.history {
        if let Ok(list) = db.list_aliases() {
            state.aliases = list;
        }
    }
}

/// 热键录制态：下一组按键即新快捷键（Esc 取消）。
fn hotkey_record_key(state: &mut State, key: Key, mods: Modifiers) -> Task<Message> {
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
    let Some(k) = key_name else { return Task::none() };
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

impl State {
    /// 对齐 commands::search_apps 的完整管线（缺内置设置页 UI，其余全量）：
    /// 空 Query 走固定+最近；非空走 内置项 → 应用召回 → 链接识别 → 文件 →
    /// 历史加权 → 网页搜索槽位；列表出全量（滚动加载在进程内直接滚动可见）。
    fn refresh_results(&mut self) {
        let t0 = Instant::now();
        let q_norm = search::normalize_for_index(&self.query);
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(db) = &self.history {
            self.pinned = db.pinned_ids().into_iter().collect();
        }
        self.results = if q_norm.is_empty() {
            let (recent, pinned) = self
                .history
                .as_ref()
                .map(|h| {
                    (
                        h.recent_ids(search::MAX_RESULTS).unwrap_or_default(),
                        h.pinned_ids(),
                    )
                })
                .unwrap_or_default();
            search::order_by_recent(&index.apps, &recent, &pinned, search::MAX_RESULTS)
        } else {
            let user_targets: Vec<search::UserTarget> = self
                .history
                .as_ref()
                .map(|h| {
                    h.alias_matches(&q_norm)
                        .into_iter()
                        .map(|a| search::UserTarget {
                            id: a.target_id,
                            name: a.target_name.to_lowercase(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut hits =
                search::search(&index.apps, &self.query, &user_targets, search::MAX_RESULTS);

            // 内置：Kite 设置 + Windows 系统设置页（css 对齐前端顺序：内置在前）
            let mut builtins = app::builtin::collect_builtin_hits(&q_norm, &self.icon_dir);
            builtins.append(&mut hits);
            hits = builtins;

            // 链接识别：网址 → 列已装浏览器直达（偏好优先）
            let preferred = self.history.as_ref().and_then(|h| h.preferred_browser());
            let search_template = self.history.as_ref().and_then(|h| h.search_url_template());
            let is_url = search::url::normalize_url(&self.query).is_some();
            if let Some(url) = search::url::normalize_url(&self.query) {
                let mut merged = app::web::build_hits(&url, preferred.as_deref(), &self.icon_dir);
                merged.append(&mut hits);
                hits = merged;
            }

            // Everything 文件搜索：仅「文件」开关打开且 query ≥ 2 字符时调用（IPC 上限 20）
            if self.files_mode && q_norm.chars().count() >= 2 {
                for fh in system::everything::search_files(&self.query, 20) {
                    let name = fh.name.clone();
                    let id = format!("file:{}", fh.path.to_lowercase());
                    let mut item = AppItem::scanned(id, name, fh.path, None, None, "everything");
                    item.attach_search_fields();
                    item.icon =
                        system::icons::cache_type_icon(&self.icon_dir, &fh.name, fh.is_folder);
                    hits.push(SearchResult {
                        item,
                        score: 400,
                        matched_by: "file".into(),
                    });
                }
            }

            // 个性化加权（历史 + 固定，Match 仍是主信号）
            if let Some(db) = &self.history {
                let ids: Vec<String> = hits.iter().map(|h| h.item.id.clone()).collect();
                let usage = db.usage_snapshot(&ids);
                let pairs = db.query_pair_snapshot(&q_norm, &ids);
                let pinned = db.pinned_ids().into_iter().collect();
                history::apply_boosts(&mut hits, usage, pairs, &q_norm, storage::now_ts(), &pinned);
            }
            hits = search::rerank(hits, search::MAX_RESULTS);

            // 网页搜索：非网址；有应用类结果时第 5 位固定「用浏览器搜索」，否则列浏览器
            if !is_url && !self.query.trim().is_empty() {
                let has_app_like = hits
                    .iter()
                    .any(|h| h.item.source != "browser" && h.item.source != "websearch");
                if has_app_like {
                    if let Some(web) = app::web::build_primary_search_hit(
                        self.query.trim(),
                        preferred.as_deref(),
                        &self.icon_dir,
                        search_template.as_deref(),
                    ) {
                        hits = app::web::insert_at_slot(hits, web, app::web::WEB_SEARCH_SLOT);
                    }
                } else {
                    hits = app::web::build_search_hits(
                        self.query.trim(),
                        preferred.as_deref(),
                        &self.icon_dir,
                        search_template.as_deref(),
                    );
                    hits = search::rerank(hits, search::MAX_RESULTS);
                }
            }

            // 成功嗅探到引擎模板则写回（副本库），避免每次读浏览器配置
            if search_template.is_none() {
                if let Some(pref) = preferred.as_deref() {
                    if let Some(t) = system::search_engine::detect_search_template(pref) {
                        if let Some(db) = self.history.as_mut() {
                            let _ = db.set_search_url_template(&t);
                        }
                    }
                }
            }

            hits
        };
        self.selected = 0;
        let top = self
            .results
            .first()
            .map(|r| r.item.display_name.as_str())
            .unwrap_or("-");
        plog(&format!(
            "query '{:?}' -> {} results in {}us top='{top}' epoch={}",
            self.query,
            self.results.len(),
            t0.elapsed().as_micros(),
            self.epoch
        ));
    }
}

/// 视图层：设置页 / 搜索页（样式在 ui.rs 与 settings_ui.rs）。
fn view(state: &State) -> iced::Element<'_, Message> {
    if state.settings_open {
        settings_view::settings_view(state)
    } else {
        search_view::view(state)
    }
}
