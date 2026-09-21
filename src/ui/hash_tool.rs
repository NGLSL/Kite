//! Hash 独立工具窗：输入 UTF-8 文本，实时计算 SHA-256。

use iced::widget::text_editor;
use iced::{Element, Font};
use sha2::{Digest, Sha256};

use super::tool_template::{self, Header, NoteTone};
use super::tools::{ToolKind, ToolOp};
use super::{Message, State};

pub(super) const TOOL_W: f32 = 880.0;
pub(super) const TOOL_H: f32 = 560.0;

pub(super) fn is_native_hash_activation(act: &crate::plugin::Activation) -> bool {
    act.plugin_id == "com.kite.devtools" && act.provider_id == "hash"
}

pub(super) fn input_text(content: &text_editor::Content) -> String {
    content.text()
}

pub(super) fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

fn tool_msg(op: ToolOp) -> Message {
    Message::Tool(ToolKind::Hash, op)
}

fn tool_edit(action: text_editor::Action) -> Message {
    Message::Tool(ToolKind::Hash, ToolOp::Edit(action))
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let win = state.tools.get(ToolKind::Hash);
    let tokens = state.theme_tokens();
    let toolbar = tool_template::toolbar(vec![
        tool_template::button("从剪贴板粘贴", tool_msg(ToolOp::Paste), false, tokens),
        tool_template::button("复制 SHA-256", tool_msg(ToolOp::CopyResult), false, tokens),
        tool_template::button("清空", tool_msg(ToolOp::Clear), false, tokens),
    ]);

    let note = match &win.note {
        Some((is_error, message)) => tool_template::note(
            message.clone(),
            if *is_error {
                NoteTone::Error
            } else {
                NoteTone::Success
            },
            tokens,
        ),
        None => tool_template::note(
            "输入变化后自动计算 · 结果为 64 位十六进制字符串",
            NoteTone::Hint,
            tokens,
        ),
    };

    let left = tool_template::editor_pane(
        "原始文本",
        tool_template::editor(
            &win.editor,
            "在这里输入或粘贴要计算 Hash 的文本…",
            tool_edit,
            Font::MONOSPACE,
            tokens,
        ),
        tokens,
    );
    let right = tool_template::result_pane(
        "SHA-256 结果",
        if win.result.is_empty() {
            "输入文本后自动计算；空文本不显示结果".into()
        } else {
            win.result.clone()
        },
        win.result.is_empty(),
        Font::MONOSPACE,
        tokens,
    );

    tool_template::window(
        tokens,
        Header {
            icon: "#",
            title: "Hash 工具",
            subtitle: "SHA-256 · UTF-8 文本",
            drag: tool_msg(ToolOp::Drag),
            close: tool_msg(ToolOp::Close),
        },
        toolbar,
        note,
        tool_template::panes(left, right),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_value_and_editor_newline() {
        let editor = text_editor::Content::with_text("abc");
        assert_eq!(input_text(&editor), "abc");
        assert_eq!(
            sha256_hex(&input_text(&editor)),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            input_text(&text_editor::Content::with_text("abc\n")),
            "abc\n"
        );
    }
}
