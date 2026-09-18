//! 通用：开机自启 / 失焦隐藏 / 历史 / 诊断日志。

use iced::Element;

use super::super::{Message, State};
use super::widgets::{flow_card, flow_row, std_button, toggle};

pub(super) fn general_card(state: &State) -> Element<'_, Message> {
    flow_card(vec![
        flow_row(
            "开机自动启动",
            "登录 Windows 后在后台待命".to_string(),
            toggle(state.autostart, Message::SetAutostart(!state.autostart)),
        ),
        flow_row(
            "失焦时隐藏",
            "点击其它窗口后自动收起启动器".to_string(),
            toggle(
                state.hide_on_blur,
                Message::SetHideOnBlur(!state.hide_on_blur),
            ),
        ),
        flow_row(
            "记录使用历史",
            "暂停后不再记录启动次数与查询偏好".to_string(),
            toggle(
                state.history_recording,
                Message::SetHistoryRecording(!state.history_recording),
            ),
        ),
        flow_row(
            "查询与按键诊断日志",
            "关闭后键入与查询不再写日志，排查时再打开".to_string(),
            toggle(state.query_log, Message::SetQueryLog(!state.query_log)),
        ),
        flow_row(
            "清空使用历史",
            "删除全部启动次数与查询配对，固定项不受影响".to_string(),
            std_button("清空", Message::ClearHistory),
        ),
    ])
}
