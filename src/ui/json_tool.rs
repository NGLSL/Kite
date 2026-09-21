//! JSON 独立工具窗：真正的第二个 OS 窗口，不改主启动器窗口。
//!
//! 产品形态：选择 JSON 工具或搜索 `json` 后弹出独立工具窗——
//! 标题栏 + 双栏（左原始 / 右结果）；关闭只销毁工具窗，主窗保持原样。

use iced::font::{Family, Font};
use iced::Element;

use super::font::ui_font;
use super::tool_template::{self, Header, NoteTone};
use super::tools::{ToolKind, ToolOp};
use super::{Message, State};

/// 工具窗逻辑尺寸（独立于启动器 640×420 / 设置 720×520）。
pub(super) const TOOL_W: f32 = 880.0;
pub(super) const TOOL_H: f32 = 560.0;

/// 搜索列表：JSON 工具条目；Enter 打开。
pub(super) const CONFIRM_RESULT_ID: &str = "kite:json-tool-confirm";

/// 官方 DevTools 的 json Provider：宿主原生工具窗，需二次确认后打开。
pub(super) fn is_native_json_activation(act: &crate::plugin::Activation) -> bool {
    act.plugin_id == "com.kite.devtools" && act.provider_id == "json"
}

/// 解析/序列化 JSON：pretty 或 minify。
pub(super) fn transform_json(raw: &str, minify: bool) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请在左侧粘贴或输入 JSON".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| format!("JSON 无效：{e}"))?;
    if minify {
        serde_json::to_string(&value).map_err(|e| format!("序列化失败：{e}"))
    } else {
        serde_json::to_string_pretty(&value).map_err(|e| format!("序列化失败：{e}"))
    }
}

fn tool_msg(op: ToolOp) -> Message {
    Message::Tool(ToolKind::Json, op)
}

fn tool_edit(action: iced::widget::text_editor::Action) -> Message {
    Message::Tool(ToolKind::Json, ToolOp::Edit(action))
}

pub fn view(state: &State) -> Element<'_, Message> {
    let win = state.tools.get(ToolKind::Json);
    let tokens = state.theme_tokens();
    let toolbar = tool_template::toolbar(vec![
        tool_template::button("从剪贴板粘贴", tool_msg(ToolOp::Paste), false, tokens),
        tool_template::button(
            "格式化",
            tool_msg(ToolOp::Transform { minify: false }),
            true,
            tokens,
        ),
        tool_template::button(
            "压缩",
            tool_msg(ToolOp::Transform { minify: true }),
            false,
            tokens,
        ),
        tool_template::button("复制结果", tool_msg(ToolOp::CopyResult), false, tokens),
        tool_template::button("清空", tool_msg(ToolOp::Clear), false, tokens),
    ]);

    let note = match &win.note {
        Some((is_err, msg)) => tool_template::note(
            msg.clone(),
            if *is_err {
                NoteTone::Error
            } else {
                NoteTone::Success
            },
            tokens,
        ),
        None => tool_template::no_note(),
    };

    let mono = Font {
        family: Family::Name("Consolas"),
        ..ui_font()
    };

    let left = tool_template::editor_pane(
        "原始 JSON",
        tool_template::editor(
            &win.editor,
            "粘贴或输入 JSON…",
            tool_edit,
            mono,
            tokens,
        ),
        tokens,
    );
    let right = tool_template::result_pane(
        "结果",
        if win.result.is_empty() {
            "结果会显示在这里".into()
        } else {
            win.result.clone()
        },
        win.result.is_empty(),
        mono,
        tokens,
    );

    tool_template::window(
        tokens,
        Header {
            icon: "{ }",
            title: "JSON 工具",
            subtitle: "开发者工具 · 原生独立窗",
            drag: tool_msg(ToolOp::Drag),
            close: tool_msg(ToolOp::Close),
        },
        toolbar,
        note,
        tool_template::panes(left, right),
    )
}
