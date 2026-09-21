//! 独立开发者工具窗：共享类型与消息处理。
//!
//! JSON / Hash / Base64 共用同一套窗口状态与操作枚举；视觉壳层在 `tool_template`。

use iced::widget::text_editor;
use iced::window::{self, settings::PlatformSpecific, Position, Settings};
use iced::Task;

use super::actions::{place_on_cursor_monitor_task, sync_scroll};
use super::{base64_tool, hash_tool, json_tool, Message, State};

/// 非内联工具（独立窗）打开前的二次确认载荷。
#[derive(Debug, Clone)]
pub(crate) struct PendingToolConfirm {
    /// 目前仅 JSON 走二次确认；字段保留扩展位。
    pub kind: ToolKind,
    /// 搜索 `json <payload>` 时的预填内容；空工具为 None。
    pub payload: Option<String>,
    /// true = 从设置页进入；false = 从搜索结果确认。
    pub from_settings: bool,
}

/// 工具种类。设置页与搜索激活都通过它路由，不再为每种工具复制一套 Message。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolKind {
    Json,
    Hash,
    Base64,
}

impl ToolKind {
    pub(crate) const ALL: [ToolKind; 3] = [ToolKind::Json, ToolKind::Hash, ToolKind::Base64];

    pub(crate) fn title(self) -> &'static str {
        match self {
            ToolKind::Json => "JSON 工具",
            ToolKind::Hash => "Hash 工具",
            ToolKind::Base64 => "Base64 工具",
        }
    }

    fn log_name(self) -> &'static str {
        match self {
            ToolKind::Json => "json",
            ToolKind::Hash => "hash",
            ToolKind::Base64 => "base64",
        }
    }

    fn tool_size(self) -> (f32, f32) {
        match self {
            ToolKind::Json => (json_tool::TOOL_W, json_tool::TOOL_H),
            ToolKind::Hash => (hash_tool::TOOL_W, hash_tool::TOOL_H),
            ToolKind::Base64 => (base64_tool::TOOL_W, base64_tool::TOOL_H),
        }
    }
}

/// 对某个工具窗的操作。JSON/Hash/Base64 的编辑、粘贴、复制、清空共用。
#[derive(Debug, Clone)]
pub(crate) enum ToolOp {
    Drag,
    Close,
    Edit(text_editor::Action),
    /// JSON：pretty / minify。
    Transform { minify: bool },
    /// Base64：编码 / 解码。
    Encode,
    Decode,
    Paste,
    PasteReady(Option<String>),
    CopyResult,
    Clear,
}

/// 单个工具窗的状态。
#[derive(Debug, Clone)]
pub(crate) struct ToolWindowState {
    pub window_id: Option<window::Id>,
    pub editor: text_editor::Content,
    pub result: String,
    pub note: Option<(bool, String)>,
}

impl Default for ToolWindowState {
    fn default() -> Self {
        Self {
            window_id: None,
            editor: text_editor::Content::default(),
            result: String::new(),
            note: None,
        }
    }
}

/// 三种工具窗的并集。主启动器窗口永不承载工具 UI。
#[derive(Debug, Clone, Default)]
pub(crate) struct ToolWindows {
    pub json: ToolWindowState,
    pub hash: ToolWindowState,
    pub base64: ToolWindowState,
}

impl ToolWindows {
    pub(crate) fn get(&self, kind: ToolKind) -> &ToolWindowState {
        match kind {
            ToolKind::Json => &self.json,
            ToolKind::Hash => &self.hash,
            ToolKind::Base64 => &self.base64,
        }
    }

    pub(crate) fn get_mut(&mut self, kind: ToolKind) -> &mut ToolWindowState {
        match kind {
            ToolKind::Json => &mut self.json,
            ToolKind::Hash => &mut self.hash,
            ToolKind::Base64 => &mut self.base64,
        }
    }

    pub(crate) fn any_open(&self) -> bool {
        self.json.window_id.is_some()
            || self.hash.window_id.is_some()
            || self.base64.window_id.is_some()
    }

    pub(crate) fn kind_of_window(&self, id: window::Id) -> Option<ToolKind> {
        ToolKind::ALL
            .into_iter()
            .find(|kind| self.get(*kind).window_id == Some(id))
    }
}

impl State {
    /// 任一工具窗是否打开（原 `plugin_tool_open`）。
    pub(crate) fn any_tool_open(&self) -> bool {
        self.tools.any_open()
    }
}

/// 进入打开工具的二次确认（目前仅 JSON）。
pub(super) fn open_from_settings(state: &mut State, kind: ToolKind) -> Task<Message> {
    match kind {
        ToolKind::Json => arm_confirm(state, ToolKind::Json, None, true),
        ToolKind::Hash => open_tool_panel(state, ToolKind::Hash, None, false),
        ToolKind::Base64 => open_tool_panel(state, ToolKind::Base64, None, false),
    }
}

/// 确认后真正打开工具窗。
pub(super) fn confirm_open(state: &mut State) -> Task<Message> {
    let Some(pending) = state.pending_tool_confirm.take() else {
        return Task::none();
    };
    open_tool_panel(
        state,
        pending.kind,
        pending.payload,
        !pending.from_settings,
    )
}

/// 取消二次确认。
pub(super) fn cancel_confirm(state: &mut State) -> Task<Message> {
    let from_settings = state
        .pending_tool_confirm
        .as_ref()
        .map(|p| p.from_settings)
        .unwrap_or(true);
    state.pending_tool_confirm = None;
    state.qlog(|| "tool confirm cancelled".to_owned());
    if from_settings {
        return Task::none();
    }
    state.refresh_results();
    sync_scroll(state)
}

/// 非内联工具打开前的二次确认。内联插件不走这里。
pub(super) fn arm_confirm(
    state: &mut State,
    kind: ToolKind,
    payload: Option<String>,
    from_settings: bool,
) -> Task<Message> {
    state.pending_tool_confirm = Some(PendingToolConfirm {
        kind,
        payload: payload.filter(|p| !p.trim().is_empty()),
        from_settings,
    });
    state.qlog(|| format!("{} tool confirm armed", kind.log_name()));
    if from_settings {
        return Task::none();
    }
    state.results = confirm_results(state.pending_tool_confirm.as_ref());
    state.results_stale = false;
    state.selected = 0;
    state.navigation_mode = super::NavigationMode::Input;
    state.hover_suppressed = false;
    state.provider_mode = None;
    state.plugin_panel = None;
    sync_scroll(state)
}

fn confirm_results(pending: Option<&PendingToolConfirm>) -> Vec<crate::model::SearchResult> {
    use crate::model::{AppItem, SearchResult};
    let kind = pending.map(|p| p.kind).unwrap_or(ToolKind::Json);
    let has_payload = pending
        .and_then(|p| p.payload.as_deref())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let (id, title) = match kind {
        ToolKind::Json => (json_tool::CONFIRM_RESULT_ID, "JSON 工具"),
        ToolKind::Hash => ("kite:hash-tool-confirm", "Hash 工具"),
        ToolKind::Base64 => ("kite:base64-tool-confirm", "Base64 工具"),
    };
    let sub = if has_payload {
        "独立工具窗 · 预填处理 · Enter 打开"
    } else {
        "独立工具窗 · Enter 打开"
    };
    vec![SearchResult::scored(
        AppItem::scanned(id.into(), title.into(), sub.into(), None, None, "builtin"),
        980,
        "builtin",
    )]
}

/// 打开独立工具窗；payload 非空时预填。from_search 时隐藏主启动器。
pub(super) fn open_tool_panel(
    state: &mut State,
    kind: ToolKind,
    payload: Option<String>,
    from_search: bool,
) -> Task<Message> {
    prefill_tool(state, kind, payload.as_deref());

    if let Some(tid) = state.tools.get(kind).window_id {
        state.qlog(|| format!("{} tool focus", kind.log_name()));
        return window::gain_focus(tid);
    }

    let (w, h) = kind.tool_size();
    let (tool_id, open_task) = window::open(tool_window_settings(w, h));
    state.tools.get_mut(kind).window_id = Some(tool_id);
    state.qlog(|| format!("{} tool open", kind.log_name()));

    let mut tasks = vec![open_task.then(move |_opened| {
        Task::batch([
            window::gain_focus(tool_id),
            place_on_cursor_monitor_task(tool_id, w, h),
        ])
    })];
    if from_search {
        if let Some(main) = state.window_id {
            state.hidden = true;
            tasks.push(window::set_mode(main, window::Mode::Hidden));
        }
    }
    Task::batch(tasks)
}

/// 关闭指定工具窗；只销毁工具窗，主窗尺寸/内容不动。
pub(super) fn close_tool_panel(state: &mut State, kind: ToolKind) -> Task<Message> {
    let Some(id) = state.tools.get_mut(kind).window_id.take() else {
        return Task::none();
    };
    state.qlog(|| format!("{} tool close", kind.log_name()));
    window::close(id)
}

/// 系统关闭事件：按窗口 id 清理对应工具状态。
pub(super) fn handle_window_closed(state: &mut State, id: window::Id) -> Task<Message> {
    let Some(kind) = state.tools.kind_of_window(id) else {
        return Task::none();
    };
    state.tools.get_mut(kind).window_id = None;
    state
        .qlog(|| format!("{} tool window closed", kind.log_name()));
    Task::none()
}

/// 处理 Tool(kind, op)。
pub(super) fn handle_op(state: &mut State, kind: ToolKind, op: ToolOp) -> Task<Message> {
    match op {
        ToolOp::Drag => state
            .tools
            .get(kind)
            .window_id
            .map(window::drag)
            .unwrap_or_else(Task::none),
        ToolOp::Close => close_tool_panel(state, kind),
        ToolOp::Edit(action) => handle_edit(state, kind, action),
        ToolOp::Transform { minify } => {
            if kind == ToolKind::Json {
                apply_json_transform(state, minify);
            }
            Task::none()
        }
        ToolOp::Encode => {
            if kind == ToolKind::Base64 {
                let raw = state.tools.base64.editor.text();
                if raw.is_empty() {
                    state.tools.base64.result.clear();
                    state.tools.base64.note = Some((true, "请先输入要编码的文本".into()));
                } else {
                    state.tools.base64.result = base64_tool::encode_utf8(&raw);
                    state.tools.base64.note = Some((false, "已编码为标准 Base64".into()));
                }
            }
            Task::none()
        }
        ToolOp::Decode => {
            if kind == ToolKind::Base64 {
                match base64_tool::decode_utf8(&state.tools.base64.editor.text()) {
                    Ok(decoded) => {
                        state.tools.base64.result = decoded;
                        state.tools.base64.note = Some((false, "已解码为 UTF-8 文本".into()));
                    }
                    Err(error) => {
                        state.tools.base64.result.clear();
                        state.tools.base64.note = Some((true, error));
                    }
                }
            }
            Task::none()
        }
        ToolOp::Paste => {
            let kind = kind;
            iced::clipboard::read().map(move |text| Message::Tool(kind, ToolOp::PasteReady(text)))
        }
        ToolOp::PasteReady(text) => handle_paste_ready(state, kind, text),
        ToolOp::CopyResult => handle_copy_result(state, kind),
        ToolOp::Clear => {
            let win = state.tools.get_mut(kind);
            win.editor = text_editor::Content::default();
            win.result.clear();
            win.note = None;
            Task::none()
        }
    }
}

fn handle_edit(state: &mut State, kind: ToolKind, action: text_editor::Action) -> Task<Message> {
    match kind {
        ToolKind::Json => {
            state.tools.json.editor.perform(action);
            Task::none()
        }
        ToolKind::Hash => {
            state.tools.hash.editor.perform(action);
            recompute_hash(state);
            state.tools.hash.note = None;
            Task::none()
        }
        ToolKind::Base64 => {
            let previous = action.is_edit().then(|| state.tools.base64.editor.text());
            state.tools.base64.editor.perform(action);
            if previous.is_some_and(|text| text != state.tools.base64.editor.text()) {
                state.tools.base64.result.clear();
                state.tools.base64.note = None;
            }
            Task::none()
        }
    }
}

fn handle_paste_ready(
    state: &mut State,
    kind: ToolKind,
    text: Option<String>,
) -> Task<Message> {
    match kind {
        ToolKind::Json => match text {
            Some(raw) if !raw.trim().is_empty() => {
                state.tools.json.editor = text_editor::Content::with_text(&raw);
                state.tools.json.result.clear();
                state.tools.json.note = None;
            }
            _ => {
                state.tools.json.note = Some((true, "剪贴板为空".into()));
            }
        },
        ToolKind::Hash => {
            if let Some(raw) = text {
                state.tools.hash.editor = text_editor::Content::with_text(&raw);
                recompute_hash(state);
                state.tools.hash.note = None;
            } else {
                state.tools.hash.note = Some((true, "剪贴板为空".into()));
            }
        }
        ToolKind::Base64 => {
            if let Some(raw) = text.filter(|s| !s.is_empty()) {
                state.tools.base64.editor = text_editor::Content::with_text(&raw);
                state.tools.base64.result.clear();
                state.tools.base64.note = None;
            } else {
                state.tools.base64.note = Some((true, "剪贴板为空".into()));
            }
        }
    }
    Task::none()
}

fn handle_copy_result(state: &mut State, kind: ToolKind) -> Task<Message> {
    let win = state.tools.get_mut(kind);
    if win.result.trim().is_empty() {
        let msg = match kind {
            ToolKind::Hash => "请先输入文本",
            _ => "右侧没有可复制的结果",
        };
        win.note = Some((true, msg.into()));
        return Task::none();
    }
    let success = match kind {
        ToolKind::Hash => "已复制 SHA-256",
        _ => "已复制结果",
    };
    win.note = Some((false, success.into()));
    iced::clipboard::write(win.result.clone())
}

fn recompute_hash(state: &mut State) {
    let input = hash_tool::input_text(&state.tools.hash.editor);
    state.tools.hash.result = if input.is_empty() {
        String::new()
    } else {
        hash_tool::sha256_hex(&input)
    };
}

fn apply_json_transform(state: &mut State, minify: bool) {
    let raw = state.tools.json.editor.text();
    match json_tool::transform_json(&raw, minify) {
        Ok(out) => {
            state.tools.json.result = out;
            state.tools.json.note = Some((
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
                state.tools.json.result.clear();
            }
            state.tools.json.note = Some((true, msg));
        }
    }
}

fn prefill_tool(state: &mut State, kind: ToolKind, payload: Option<&str>) {
    let Some(raw) = payload.filter(|s| !s.trim().is_empty()) else {
        return;
    };
    match kind {
        ToolKind::Json => {
            state.tools.json.editor = text_editor::Content::with_text(raw);
            match json_tool::transform_json(raw, false) {
                Ok(out) => {
                    state.tools.json.result = out;
                    state.tools.json.note = Some((false, "已格式化".into()));
                }
                Err(msg) => {
                    state.tools.json.result.clear();
                    state.tools.json.note = Some((true, msg));
                }
            }
        }
        ToolKind::Hash => {
            state.tools.hash.editor = text_editor::Content::with_text(raw);
            state.tools.hash.result = hash_tool::sha256_hex(raw);
            state.tools.hash.note = None;
        }
        ToolKind::Base64 => {
            state.tools.base64.editor = text_editor::Content::with_text(raw);
            state.tools.base64.result = base64_tool::encode_utf8(raw);
            state.tools.base64.note = Some((false, "已编码".into()));
        }
    }
}

/// 三种工具窗共用的无边框独立窗设置。
fn tool_window_settings(w: f32, h: f32) -> Settings {
    Settings {
        size: iced::Size::new(w, h),
        position: Position::SpecificWith(|win, monitor| {
            iced::Point::new(
                (monitor.width - win.width) / 2.0,
                (monitor.height - win.height) / 2.0,
            )
        }),
        visible: true,
        resizable: false,
        decorations: false,
        level: window::Level::Normal,
        exit_on_close_request: false,
        platform_specific: PlatformSpecific {
            skip_taskbar: false,
            undecorated_shadow: false,
            corner_preference: iced::window::settings::platform::CornerPreference::Round,
            ..PlatformSpecific::default()
        },
        ..Settings::default()
    }
}
