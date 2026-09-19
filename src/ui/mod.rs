//! 原生主界面（iced + tiny-skia 软渲染，无 WebView2）。
//! 此处定义共享状态与消息，启动、键盘、交互、结果刷新和动作分别由子模块负责。
//!
//! 日志：%APPDATA%\com.kite.launcher\kite.log；数据目录与旧版 Kite 一致，
//! 历史库/设置直接读写真实数据。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use iced::keyboard::key::Physical;
use iced::keyboard::{key::Named, Key, Modifiers};
use iced::widget::operation::{focus, scroll_to};
use iced::widget::scrollable::AbsoluteOffset;
use iced::widget::Id as WidgetId;
use iced::window::settings::PlatformSpecific;
use iced::window::{self, Position, Settings};
use iced::{stream, Subscription, Task, Theme};

use crate::model::{AppIndex, AppItem, SearchResult};
use crate::plugin::{self, Activation, PanelData, PluginHost, PluginRegistry};
use crate::storage::settings::UserAlias;
use crate::storage::HistoryDb;
use crate::system::hotkey::parse_raw;
use crate::{app, history, log, search, storage, system};

mod actions;
mod backend;
mod font;
mod interaction;
mod json_tool;
mod keyboard;
mod results;
mod runtime;
mod search_view;
mod settings;
#[cfg(test)]
mod test_support;
pub mod theme;
mod tray;

use actions::*;
use interaction::update;
use keyboard::{alt_digit_from_query_change, alt_digit_index};
pub use runtime::run;
pub use theme::{ThemeMode, ThemeTokens};

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
    SearchEngine,
    Hotkey,
    Alias,
    Index,
    Plugins,
    About,
}

/// 键盘交互的逻辑焦点。实际的 Iced widget focus 始终留在搜索框，
/// 这里仅决定方向键是在编辑 Query 还是漫游结果列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationMode {
    Input,
    Results,
}

impl Section {
    const ALL: [Section; 7] = [
        Section::General,
        Section::SearchEngine,
        Section::Hotkey,
        Section::Alias,
        Section::Index,
        Section::Plugins,
        Section::About,
    ];
    fn label(self) -> &'static str {
        match self {
            Section::General => "通用",
            Section::SearchEngine => "搜索引擎",
            Section::Hotkey => "热键",
            Section::Alias => "别名",
            Section::Index => "应用索引",
            Section::Plugins => "插件",
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

impl State {
    /// 用户交互路径上的诊断日志：**按键频率**的那些（Alt 按下/抬起、IME 组合/提交、
    /// 查询刷新、应用与文件搜索的结果落地/过期、Alt+数字）以及由窗口内键盘/指针
    /// 触发的一次性动作（Alt+数字、Enter 启动、Esc 隐藏、右键菜单、文件模式切换、
    /// 设置页开合与其快捷键/自启/更新等命令）。
    ///
    /// 开关关闭时直接返回：既不构造消息也不落盘，所以窗口内的输入处理路径上不再有
    /// 同步文件写入。窗口与热键生命周期（唤起/隐藏/blur/二次激活）、后台子系统
    /// （引导、扫描、索引、图标、托盘、更新下载线程）仍走 `plog`：它们不由窗口内
    /// 输入触发，也不随按键重复。本方法不参与结果、顺序或缓存的任何判定。
    pub(crate) fn qlog(&self, message: impl FnOnce() -> String) {
        if self.query_log {
            plog(&message());
        }
    }

    pub(crate) fn theme_tokens(&self) -> ThemeTokens {
        self.theme_mode.tokens()
    }
}

#[derive(Debug, Clone)]
enum Message {
    /// 全局快捷键触发；携带接收线程的单调时间戳（埋点起点）。
    Hotkey(Instant),
    /// 按键：逻辑键 + 物理键 + 修饰键（Alt+N 在 Windows 上逻辑键常被改写，需物理键兜底）。
    KeyPressed(Key, Physical, Modifiers),
    /// 按键抬起（跟踪 Alt 状态，避免依赖 modifiers.alt() 在 SYSKEY 下的不可靠性）。
    KeyReleased(Key, Modifiers),
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
    FileFilterChanged(system::everything::FileFilter),
    /// 后台 Everything 查询完成；代际和查询文本用于丢弃过期结果。
    FileSearchReady(u64, String, Vec<SearchResult>, u128),
    /// 后台应用搜索完成；代际和查询文本用于丢弃过期结果；末位为索引代际。
    AppSearchReady(u64, String, Vec<SearchResult>, u128, u64),
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
    /// 右键动作携带点击时的条目快照，避免后台刷新后同一行号指向另一项。
    MenuAction(AppItem, MenuAction),
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
    SetThemeMode(ThemeMode),
    /// 开关查询级诊断日志（结果就绪／过期）。
    SetQueryLog(bool),
    /// 搜索引擎预设：auto / baidu / bing / google / duckduckgo / custom。
    SetSearchEngine(String),
    /// 自定义搜索引擎 URL 模板（含 {searchTerms}）。
    SearchEngineCustomChanged(String),
    /// 窗口内网页搜索快捷键（Ctrl+Enter 等）。
    SetWebSearchHotkey(String),
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
    PortableDirInputChanged(String),
    AddPortableDir,
    RemovePortableDir(usize),
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
    /// 后台完整扫描完成并已原子替换快照。
    FullIndexReady(usize),
    /// Cold Bootstrap 已发布可搜索索引；必须提升代际并作废旧缓存。
    BootstrapReady(usize),
    /// 二次启动：请求主实例显示窗口（已显示则只抢焦点，不切换隐藏）。
    EnsureVisible,
    // ── Plugin System ──
    /// Provider Mode 下插件查询结果落地。
    PluginQueryReady(u64, PluginQueryPayload),
    /// 插件→宿主 host/* 调用（clipboard/open_url/open_path/hide_kite）。
    PluginHostCall(String, plugin::HostCall),
    /// 设置页：启用/禁用插件。
    PluginSetEnabled(String, bool),
    /// 设置页：重新加载插件进程。
    PluginReload(String),
    /// 设置页：打开插件目录。
    PluginOpenDir(String),
    /// 设置页：打开插件 stderr 日志。
    PluginOpenLog(String),
    /// 设置页：卸载插件（删除插件根目录 + 移出 Registry）。
    PluginUninstall(String),
    /// 设置页：导入路径输入。
    PluginImportPathChanged(String),
    /// 设置页：从路径导入插件文件夹。
    PluginImportFromPath,
    /// 设置页：安装捆绑的官方示例插件。
    PluginInstallOfficial,
    /// 设置页：打开全局插件目录。
    PluginOpenPluginsDir,
    /// 设置页：重新扫描插件目录。
    PluginRescanPlugins,
    /// 设置页/试用：把示例查询填入搜索框并回到主界面。
    PluginTryExample(String),
    /// 设置页：展开/收起某个插件的详细使用说明。
    PluginToggleDocs(String),
    /// 进入「打开 JSON 工具」二次确认（不直接开窗）。
    PluginOpenJsonTool,
    /// 确认打开 JSON 独立工具窗。
    PluginConfirmJsonTool,
    /// 取消打开 JSON 工具的二次确认。
    PluginCancelJsonToolConfirm,
    /// 关闭 JSON 独立工具窗。
    PluginCloseJsonTool,
    /// 拖拽 JSON 工具窗标题栏。
    JsonToolDrag,
    /// 某窗口已销毁；工具窗关闭时清理状态。
    JsonToolWindowClosed(window::Id),
    /// JSON 工具左侧编辑器动作。
    JsonToolEdit(iced::widget::text_editor::Action),
    /// 左侧原文 → 右侧格式化（缩进）。
    JsonToolFormat,
    /// 左侧原文 → 右侧压缩（单行）。
    JsonToolMinify,
    /// 从系统剪贴板粘贴到左侧。
    JsonToolPaste,
    /// 剪贴板读取结果落地。
    JsonToolPasteReady(Option<String>),
    /// 复制右侧结果到剪贴板。
    JsonToolCopyResult,
    /// 清空左右两侧。
    JsonToolClear,
}

/// 插件查询落地载荷（代际 + 结果）。
#[derive(Debug, Clone)]
pub(crate) enum PluginQueryPayload {
    List {
        plugin_id: String,
        provider_id: String,
        items: Vec<SearchResult>,
    },
    Panel {
        plugin_id: String,
        provider_id: String,
        panel: PanelData,
    },
    Empty {
        plugin_id: String,
        provider_id: String,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    OpenFolder,
    CopyPath,
    /// shell:AppsFolder / ms-settings 等非路径 target 的复制。
    CopyTarget,
    CopyName,
    TogglePin,
    /// 降低此结果优先级（可恢复）。
    Demote,
    /// 恢复被降权结果的默认优先级。
    Undemote,
}

struct State {
    #[allow(dead_code)]
    data_dir: PathBuf,
    /// Everything64.dll 所在资源目录（文件搜索用）。
    icon_dir: PathBuf,
    index: std::sync::Arc<Mutex<AppIndex>>,
    scan_options: std::sync::Arc<std::sync::RwLock<app::scanner::ScanOptions>>,
    history: Option<HistoryDb>,
    input_id: WidgetId,
    window_id: Option<window::Id>,
    query: String,
    results: Vec<SearchResult>,
    selected: usize,
    navigation_mode: NavigationMode,
    hidden: bool,
    ime_composing: bool,
    /// 本地跟踪的 Alt 按下态（Windows SYSKEY 下 modifiers.alt() 可能不可靠）。
    alt_down: bool,
    /// 按下 Alt 时的 Query；用于丢弃 Alt+数字被 text_input 误插入的字符。
    query_at_alt: Option<String>,
    /// 同一次 Alt 按下最多触发一次结果启动（KeyPressed 和 text_input 可能都报告数字）。
    alt_digit_consumed: bool,
    index_ready: bool,
    /// 用户主动点了重新扫描；完成后给设置页一条可见反馈。
    rescan_pending: bool,
    files_mode: bool,
    file_filter: system::everything::FileFilter,
    file_query_generation: u64,
    file_results: Vec<SearchResult>,
    /// 应用搜索 Query 代际（后台 worker 丢弃过期结果）。
    app_query_generation: u64,
    /// 已展示列表是否已过期：新查询提交后置位，结果落地后清除。
    /// 过期期间列表可继续显示（避免闪空），但 Enter / Alt+数字 / 点击都不得启动它。
    results_stale: bool,
    /// 索引代际：扫描完成后递增，用于基础命中缓存失效。
    index_generation: u64,
    /// 基础候选缓存：存**个性化前**的轻量候选，个性化每次按最新偏好重放。
    base_hit_cache: std::sync::Arc<search::service::BaseHitCache>,
    /// 常驻应用搜索 worker（全局唯一）。
    app_search_worker: std::sync::Arc<search::service::AppSearchWorker>,
    /// 右键菜单：(点击时的条目快照, x, y)。
    menu: Option<(AppItem, f32, f32)>,
    pinned: std::collections::HashSet<String>,
    /// 最新光标（窗口逻辑坐标；Cell 写入不参与视图比较）。
    cursor: std::rc::Rc<std::cell::Cell<iced::Point>>,
    // ── 设置页 ──
    settings_open: bool,
    settings_section: Section,
    hide_on_blur: bool,
    autostart: bool,
    history_recording: bool,
    pub theme_mode: ThemeMode,
    /// 空 Query 网格仪表盘中“最近使用”条目数（用于精确分区与键盘漫游）
    pub(crate) grid_recent_count: usize,
    /// 输入与查询级诊断日志开关（Alt/IME 按键日志 + 结果就绪／过期）。保存在内存里，
    /// 避免按键路径去读 SQLite。只闸住按键频率的日志；关闭后按键路径零同步写入。
    query_log: bool,
    /// 搜索引擎：auto / baidu / bing / google / duckduckgo / custom。
    search_engine: String,
    /// 自定义模板编辑框（仅 custom 时写入 DB）。
    search_engine_custom: String,
    /// 窗口内网页搜索快捷键，默认 Ctrl+Enter。
    web_search_hotkey: String,
    hotkey: String,
    hotkey_label: String,
    aliases: Vec<UserAlias>,
    alias_input: String,
    alias_target_input: String,
    alias_candidates: Vec<SearchResult>,
    alias_pick: Option<UserAlias>,
    portable_dirs: Vec<String>,
    portable_dir_input: String,
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
    /// 键盘导航后抑制悬停改选，直到鼠标实际移动（避免 scroll_to 后 on_enter 抢选中）。
    hover_suppressed: bool,
    /// 上次用于悬停判定的鼠标位置（有位移才恢复悬停选中）。
    last_hover_pt: Option<iced::Point>,
    // ── Plugin System ──
    plugin_registry: std::sync::Arc<Mutex<PluginRegistry>>,
    plugin_host: std::sync::Arc<Mutex<PluginHost>>,
    provider_mode: Option<Activation>,
    plugin_panel: Option<PanelData>,
    plugin_query_generation: u64,
    plugin_flash: Option<String>,
    /// 插件导入：设置页粘贴的本地文件夹路径。
    plugin_import_path: String,
    /// 设置页：当前展开详细说明的插件 id。
    plugin_docs_open: Option<String>,
    /// JSON 独立工具窗的窗口 id；None = 未打开。主启动器窗口永不承载工具 UI。
    json_tool_window: Option<window::Id>,
    /// JSON 工具是否打开（与 json_tool_window 同步，供逻辑/测试读取）。
    plugin_tool_open: bool,
    /// 非内联工具打开前的二次确认（JSON）。内联插件（计算器等）不进此状态。
    pending_tool_confirm: Option<PendingJsonToolConfirm>,
    /// JSON 工具左侧原文。
    json_editor: iced::widget::text_editor::Content,
    /// JSON 工具右侧结果。
    json_result: String,
    /// JSON 工具状态提示（错误/成功）。
    json_tool_note: Option<(bool, String)>,
}

/// 非内联工具（独立窗）打开前的二次确认载荷。
#[derive(Debug, Clone)]
pub(crate) struct PendingJsonToolConfirm {
    /// 搜索 `json <payload>` 时的预填内容；空工具为 None。
    pub payload: Option<String>,
    /// true = 从设置页进入；false = 从搜索结果确认。
    pub from_settings: bool,
}

/// 按窗口路由视图：工具窗 → JSON 工具；主窗 → 设置/搜索。主窗内容不因工具而切换。
fn view(state: &State, window: window::Id) -> iced::Element<'_, Message> {
    if state.json_tool_window == Some(window) {
        return json_tool::view(state);
    }
    if state.settings_open {
        settings::settings_view(state)
    } else {
        search_view::view(state)
    }
}
