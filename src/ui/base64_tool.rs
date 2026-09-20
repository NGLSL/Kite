//! Base64 独立工具窗：UTF-8 文本与标准 Base64 双向转换。

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use iced::font::{Family, Font};
use iced::Element;

use super::font::ui_font;
use super::tool_template::{self, Header, NoteTone};
use super::{Message, State};

pub(super) const TOOL_W: f32 = 880.0;
pub(super) const TOOL_H: f32 = 560.0;

pub(super) fn is_native_base64_activation(act: &crate::plugin::Activation) -> bool {
    act.plugin_id == "com.kite.devtools" && act.provider_id == "base64"
}

pub(super) fn encode_utf8(input: &str) -> String {
    STANDARD.encode(input.as_bytes())
}

pub(super) fn decode_utf8(input: &str) -> Result<String, String> {
    let compact: String = input
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if compact.is_empty() {
        return Err("请在左侧粘贴或输入 Base64 文本".into());
    }
    let bytes = STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| "Base64 无效：请检查字符和填充".to_string())?;
    String::from_utf8(bytes).map_err(|_| "解码结果不是 UTF-8 文本".to_string())
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let tokens = state.theme_tokens();
    let mono = Font {
        family: Family::Name("Consolas"),
        ..ui_font()
    };
    let toolbar = tool_template::toolbar(vec![
        tool_template::button("从剪贴板粘贴", Message::Base64ToolPaste, false, tokens),
        tool_template::button("编码", Message::Base64ToolEncode, true, tokens),
        tool_template::button("解码", Message::Base64ToolDecode, false, tokens),
        tool_template::button("复制结果", Message::Base64ToolCopyResult, false, tokens),
        tool_template::button("清空", Message::Base64ToolClear, false, tokens),
    ]);
    let note = match &state.base64_tool_note {
        Some((error, message)) => tool_template::note(
            message.clone(),
            if *error {
                NoteTone::Error
            } else {
                NoteTone::Success
            },
            tokens,
        ),
        None => tool_template::note(
            "UTF-8 文本与标准 Base64 双向转换；解码结果须为 UTF-8 文本",
            NoteTone::Hint,
            tokens,
        ),
    };
    let left = tool_template::editor_pane(
        "输入文本 / Base64",
        tool_template::editor(
            &state.base64_editor,
            "粘贴或输入文本，选择编码或解码…",
            Message::Base64ToolEdit,
            mono,
            tokens,
        ),
        tokens,
    );
    let empty = state.base64_result.is_empty();
    let right = tool_template::result_pane(
        "结果",
        if empty {
            "结果会显示在这里".to_string()
        } else {
            state.base64_result.clone()
        },
        empty,
        mono,
        tokens,
    );
    tool_template::window(
        tokens,
        Header {
            icon: "64",
            title: "Base64 工具",
            subtitle: "开发者工具 · UTF-8 文本",
            drag: Message::Base64ToolDrag,
            close: Message::PluginCloseBase64Tool,
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
    fn utf8_round_trip_and_wrapped_base64() {
        let input = "Kite 你好 🪁";
        let encoded = encode_utf8(input);
        assert_eq!(decode_utf8(&encoded), Ok(input.to_string()));
        assert_eq!(decode_utf8("YQ==\r\n"), Ok("a".to_string()));
    }

    #[test]
    fn invalid_base64_and_non_utf8_bytes_report_errors() {
        assert!(decode_utf8("").is_err());
        assert!(decode_utf8("abc!").unwrap_err().contains("Base64 无效"));
        assert!(decode_utf8("/w==").unwrap_err().contains("UTF-8"));
    }
}
