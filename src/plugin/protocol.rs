//! JSON-RPC 2.0 over stdio + Content-Length framing。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcMessage {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl JsonRpcMessage {
    pub fn request(id: u64, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: Some(method.into()),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    pub fn notification(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: None,
            method: Some(method.into()),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    pub fn to_frame(&self) -> Result<Vec<u8>, String> {
        let body = serde_json::to_string(self).map_err(|e| e.to_string())?;
        Ok(encode_frame(&body))
    }
}

pub fn encode_frame(json_body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{}", json_body.len(), json_body).into_bytes()
}

/// 从缓冲区解析完整帧；剩余字节留在 buffer。
pub fn decode_frames(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        let header_end = find_header_end(buffer);
        let Some(he) = header_end else { break };
        let header = String::from_utf8_lossy(&buffer[..he]).to_string();
        let mut content_length = None;
        for line in header.split("\r\n") {
            let lower = line.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse::<usize>().ok();
            }
        }
        let Some(len) = content_length else {
            // 无法解析：丢弃到 header_end 避免死循环
            buffer.drain(..he + 4);
            continue;
        };
        let body_start = he + 4;
        if buffer.len() < body_start + len {
            break;
        }
        let body = buffer[body_start..body_start + len].to_vec();
        buffer.drain(..body_start + len);
        out.push(String::from_utf8_lossy(&body).to_string());
    }
    out
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub struct ContentLengthCodec;

impl ContentLengthCodec {
    pub fn encode(msg: &JsonRpcMessage) -> Result<Vec<u8>, String> {
        msg.to_frame()
    }
    pub fn decode(buffer: &mut Vec<u8>) -> Vec<JsonRpcMessage> {
        decode_frames(buffer)
            .into_iter()
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect()
    }
}

pub fn plugin_initialize(
    id: u64,
    plugin_id: &str,
    kite_version: &str,
    data_dir: &str,
) -> JsonRpcMessage {
    JsonRpcMessage::request(
        id,
        "plugin/initialize",
        json!({
            "plugin_api": 1,
            "kite_version": kite_version,
            "plugin_id": plugin_id,
            "locale": "zh-CN",
            "data_dir": data_dir,
        }),
    )
}

pub fn plugin_query(
    id: u64,
    provider_id: &str,
    raw_query: &str,
    query: &str,
    generation: u64,
) -> JsonRpcMessage {
    JsonRpcMessage::request(
        id,
        "plugin/query",
        json!({
            "provider_id": provider_id,
            "raw_query": raw_query,
            "query": query,
            "generation": generation,
        }),
    )
}

pub fn plugin_execute(id: u64, action_id: &str, payload: Value) -> JsonRpcMessage {
    JsonRpcMessage::request(
        id,
        "plugin/execute",
        json!({ "action_id": action_id, "payload": payload }),
    )
}

pub fn cancel_notification(request_id: u64) -> JsonRpcMessage {
    JsonRpcMessage::notification("$/cancelRequest", json!({ "id": request_id }))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    List {
        items: Vec<ListItem>,
    },
    Panel {
        panel: super::panel::PanelData,
    },
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub action: Option<ListItemAction>,
}

/// List 条目动作：宿主用当前 plugin_id 执行 plugin/execute。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListItemAction {
    PluginAction {
        action_id: String,
        #[serde(default)]
        payload: Value,
    },
    CopyText {
        text: String,
    },
    OpenFile {
        path: String,
    },
    OpenUrl {
        url: String,
    },
    #[serde(other)]
    Unknown,
}

impl ListItemAction {
    pub fn to_result_action(&self, plugin_id: &str, item_id: &str) -> crate::model::ResultAction {
        match self {
            ListItemAction::PluginAction { action_id, payload } => {
                crate::model::ResultAction::Plugin {
                    plugin_id: plugin_id.to_string(),
                    action_id: action_id.clone(),
                    payload: payload.clone(),
                }
            }
            ListItemAction::CopyText { text } => crate::model::ResultAction::CopyText {
                text: text.clone(),
            },
            ListItemAction::OpenFile { path } => crate::model::ResultAction::OpenFile {
                path: path.clone(),
            },
            ListItemAction::OpenUrl { url } => crate::model::ResultAction::OpenUrl {
                url: url.clone(),
            },
            ListItemAction::Unknown => crate::model::ResultAction::Plugin {
                plugin_id: plugin_id.to_string(),
                action_id: "default".into(),
                payload: serde_json::json!({ "id": item_id }),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginResponse {
    pub plugin_id: String,
    pub provider_id: String,
    pub generation: u64,
    pub result: QueryResult,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostCall {
    ClipboardWrite(String),
    OpenUrl(String),
    OpenPath(String),
    HideKite,
}

/// 解析插件→宿主 `host/*` RPC。非 host 命名空间返回 None。
pub fn parse_host_call(method: &str, params: Option<&Value>) -> Option<HostCall> {
    let p = params.cloned().unwrap_or(Value::Null);
    let text_param = |key: &str| -> Option<String> {
        p.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| p.as_str().map(|s| s.to_string()))
    };
    match method {
        "host/clipboard.write" => Some(HostCall::ClipboardWrite(text_param("text")?)),
        "host/open_url" => Some(HostCall::OpenUrl(text_param("url")?)),
        "host/open_path" => Some(HostCall::OpenPath(text_param("path")?)),
        "host/hide_kite" => Some(HostCall::HideKite),
        _ => None,
    }
}

/// 从 plugin/query 的 result Value 解析 QueryResult。
pub fn parse_query_result(value: &Value) -> Result<QueryResult, String> {
    let ty = value
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or("missing result type")?;
    match ty {
        "list" => {
            #[derive(Deserialize)]
            struct ListWrap {
                items: Vec<ListItem>,
            }
            let wrap: ListWrap =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            Ok(QueryResult::List { items: wrap.items })
        }
        "panel" => {
            let panel = super::panel::parse_panel(value)?;
            Ok(QueryResult::Panel { panel })
        }
        "empty" => Ok(QueryResult::Empty),
        other => Err(format!("unknown query result type: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_length_roundtrip() {
        let msg = plugin_query(10, "calculate", "=1+2", "1+2", 108);
        let frame = msg.to_frame().unwrap();
        assert!(frame.starts_with(b"Content-Length:"));
        let mut buf = frame;
        let decoded = ContentLengthCodec::decode(&mut buf);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].method.as_deref(), Some("plugin/query"));
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_handles_partial_frames() {
        let m1 = plugin_initialize(1, "com.kite.calculator", "0.3.0", "d");
        let m2 = plugin_query(2, "calculate", "=1", "1", 1);
        let mut buf = m1.to_frame().unwrap();
        let part = m2.to_frame().unwrap();
        buf.extend_from_slice(&part[..part.len() / 2]);
        let first = ContentLengthCodec::decode(&mut buf);
        assert_eq!(first.len(), 1);
        buf.extend_from_slice(&part[part.len() / 2..]);
        let rest = ContentLengthCodec::decode(&mut buf);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, Some(2));
    }

    #[test]
    fn parse_list_and_panel_results() {
        let list = json!({
            "type": "list",
            "items": [{
                "id": "w1",
                "title": "Visual Studio Code",
                "subtitle": "Kite — Visual Studio Code",
                "priority": 10,
                "action": {
                    "type": "plugin_action",
                    "action_id": "activate_window",
                    "payload": { "hwnd": 123 }
                }
            }]
        });
        match parse_query_result(&list).unwrap() {
            QueryResult::List { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].title, "Visual Studio Code");
                match items[0].action.as_ref().unwrap() {
                    ListItemAction::PluginAction { action_id, .. } => {
                        assert_eq!(action_id, "activate_window");
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }

        let panel = json!({
            "type": "panel",
            "panel": {
                "blocks": [
                    { "type": "text", "text": "100 × 1.13", "style": "secondary" },
                    { "type": "value", "value": "113" }
                ],
                "actions": [{
                    "id": "copy",
                    "label": "复制结果",
                    "shortcut": "Enter",
                    "default": true,
                    "action": { "type": "copy_text", "text": "113" }
                }]
            }
        });
        match parse_query_result(&panel).unwrap() {
            QueryResult::Panel { panel } => {
                assert_eq!(panel.blocks.len(), 2);
                assert_eq!(panel.actions.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(parse_query_result(&json!({"type":"empty"})).unwrap(), QueryResult::Empty);
    }

    #[test]
    fn parse_host_calls() {
        assert_eq!(
            parse_host_call("host/clipboard.write", Some(&json!({"text":"3"}))),
            Some(HostCall::ClipboardWrite("3".into()))
        );
        assert_eq!(
            parse_host_call("host/open_url", Some(&json!({"url":"https://a.com"}))),
            Some(HostCall::OpenUrl("https://a.com".into()))
        );
        assert_eq!(
            parse_host_call("host/open_path", Some(&json!({"path":r"C:\tmp\a.txt"}))),
            Some(HostCall::OpenPath(r"C:\tmp\a.txt".into()))
        );
        assert_eq!(parse_host_call("host/hide_kite", None), Some(HostCall::HideKite));
        assert_eq!(parse_host_call("plugin/query", Some(&json!({}))), None);
        assert_eq!(parse_host_call("host/open_url", Some(&json!({}))), None);
    }
}
