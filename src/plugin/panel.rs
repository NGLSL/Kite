//! 声明式 Panel Schema（禁止 HTML/CSS/JS/WebView）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelData {
    #[serde(default)]
    pub blocks: Vec<PanelBlock>,
    #[serde(default)]
    pub actions: Vec<PanelAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelBlock {
    Text {
        text: String,
        #[serde(default = "default_style")]
        style: String,
    },
    Value {
        #[serde(default)]
        label: Option<String>,
        value: String,
        #[serde(default)]
        selectable: Option<bool>,
    },
    KeyValue {
        items: Vec<KeyValueItem>,
    },
    Notice {
        level: String,
        text: String,
    },
    Divider,
}

fn default_style() -> String {
    "normal".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyValueItem {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelAction {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub action: Option<ResultActionDto>,
}

/// 与 model::ResultAction 对齐的 DTO（协议层）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResultActionDto {
    LaunchApp {
        item_id: String,
    },
    OpenFile {
        path: String,
    },
    OpenUrl {
        url: String,
    },
    CopyText {
        text: String,
    },
    Plugin {
        /// 可省略：由宿主用当前 plugin_id 回填。
        #[serde(default)]
        plugin_id: String,
        action_id: String,
        #[serde(default)]
        payload: Value,
    },
    #[serde(other)]
    Unknown,
}

impl ResultActionDto {
    pub fn to_model(&self, fallback_plugin: Option<&str>) -> Option<crate::model::ResultAction> {
        match self {
            // V1 NativeAction 仅 copy_text / open_url / open_path；
            // 插件不得借 Panel 启动 Core 索引应用。
            ResultActionDto::LaunchApp { .. } => None,
            ResultActionDto::OpenFile { path } => Some(crate::model::ResultAction::OpenFile {
                path: path.clone(),
            }),
            ResultActionDto::OpenUrl { url } => Some(crate::model::ResultAction::OpenUrl {
                url: url.clone(),
            }),
            ResultActionDto::CopyText { text } => Some(crate::model::ResultAction::CopyText {
                text: text.clone(),
            }),
            ResultActionDto::Plugin {
                plugin_id,
                action_id,
                payload,
            } => {
                let pid = if plugin_id.is_empty() {
                    fallback_plugin.unwrap_or_default().to_string()
                } else {
                    plugin_id.clone()
                };
                Some(crate::model::ResultAction::Plugin {
                    plugin_id: pid,
                    action_id: action_id.clone(),
                    payload: payload.clone(),
                })
            }
            ResultActionDto::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelError {
    Invalid(String),
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PanelError::Invalid(s) => write!(f, "invalid panel: {s}"),
        }
    }
}

/// 解析 panel 载荷：`{"type":"panel","panel":{...}}` 或直接 `{blocks,actions}`。
pub fn parse_panel(value: &Value) -> Result<PanelData, String> {
    let panel_val = if value.get("type").and_then(|t| t.as_str()) == Some("panel") {
        value.get("panel").cloned().unwrap_or(Value::Null)
    } else {
        value.clone()
    };
    if panel_val.is_null() {
        return Err("missing panel".into());
    }
    let blocks_raw = panel_val
        .get("blocks")
        .and_then(|b| b.as_array())
        .ok_or("panel.blocks required")?;
    let mut blocks = Vec::new();
    for b in blocks_raw {
        let ty = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match ty {
            "text" => {
                let text = b
                    .get("text")
                    .and_then(|t| t.as_str())
                    .ok_or("text block missing text")?
                    .to_string();
                let style = b
                    .get("style")
                    .and_then(|s| s.as_str())
                    .unwrap_or("normal")
                    .to_string();
                if !matches!(style.as_str(), "normal" | "secondary" | "muted" | "error") {
                    return Err(format!("invalid text style: {style}"));
                }
                blocks.push(PanelBlock::Text { text, style });
            }
            "value" => {
                let value_s = b
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or("value block missing value")?
                    .to_string();
                let label = b
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let selectable = b.get("selectable").and_then(|v| v.as_bool());
                blocks.push(PanelBlock::Value {
                    label,
                    value: value_s,
                    selectable,
                });
            }
            "key_value" => {
                let items_raw = b
                    .get("items")
                    .and_then(|i| i.as_array())
                    .ok_or("key_value missing items")?;
                let mut items = Vec::new();
                for it in items_raw {
                    let key = it
                        .get("key")
                        .and_then(|k| k.as_str())
                        .ok_or("kv key")?
                        .to_string();
                    let value = it
                        .get("value")
                        .and_then(|k| k.as_str())
                        .ok_or("kv value")?
                        .to_string();
                    items.push(KeyValueItem { key, value });
                }
                blocks.push(PanelBlock::KeyValue { items });
            }
            "notice" => {
                let level = b
                    .get("level")
                    .and_then(|l| l.as_str())
                    .unwrap_or("info")
                    .to_string();
                if !matches!(level.as_str(), "info" | "warning" | "error") {
                    return Err(format!("invalid notice level: {level}"));
                }
                let text = b
                    .get("text")
                    .and_then(|t| t.as_str())
                    .ok_or("notice missing text")?
                    .to_string();
                blocks.push(PanelBlock::Notice { level, text });
            }
            "divider" => blocks.push(PanelBlock::Divider),
            other => return Err(format!("unsupported panel block: {other}")),
        }
    }

    let mut actions = Vec::new();
    if let Some(arr) = panel_val.get("actions").and_then(|a| a.as_array()) {
        for a in arr {
            let id = a
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("action id")?
                .to_string();
            let label = a
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(id.as_str())
                .to_string();
            let shortcut = a
                .get("shortcut")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let default = a
                .get("default")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let action = a.get("action").map(|av| {
                serde_json::from_value::<ResultActionDto>(av.clone())
                    .unwrap_or(ResultActionDto::Unknown)
            });
            actions.push(PanelAction {
                id,
                label,
                shortcut,
                default,
                action,
            });
        }
    }
    Ok(PanelData { blocks, actions })
}

/// Native 动作：Kite 直接执行，不走 plugin/execute。
#[derive(Debug, Clone, PartialEq)]
pub enum NativeAction {
    CopyText(String),
    OpenUrl(String),
    OpenPath(String),
}

impl PanelData {
    pub fn default_native_action(&self) -> Option<NativeAction> {
        let act = self.actions.iter().find(|a| a.default)?;
        match act.action.as_ref()? {
            ResultActionDto::CopyText { text } => Some(NativeAction::CopyText(text.clone())),
            ResultActionDto::OpenUrl { url } => Some(NativeAction::OpenUrl(url.clone())),
            ResultActionDto::OpenFile { path } => Some(NativeAction::OpenPath(path.clone())),
            _ => None,
        }
    }

    pub fn default_plugin_action(&self) -> Option<crate::model::ResultAction> {
        let act = self.actions.iter().find(|a| a.default)?;
        act.action.as_ref().and_then(|dto| dto.to_model(None))
    }

    /// 提取主值（优先 default copy_text，其次首个 Value 块），便于快捷复制
    pub fn primary_value(&self) -> Option<String> {
        self.default_native_action()
            .and_then(|a| match a {
                NativeAction::CopyText(t) => Some(t),
                _ => None,
            })
            .or_else(|| {
                self.blocks.iter().find_map(|b| match b {
                    PanelBlock::Value { value, .. } => Some(value.clone()),
                    _ => None,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_calculator_panel() {
        let v = json!({
            "type": "panel",
            "panel": {
                "blocks": [
                    { "type": "text", "text": "1 + 2", "style": "secondary" },
                    { "type": "value", "value": "3" }
                ],
                "actions": [{
                    "id": "copy",
                    "label": "复制结果",
                    "shortcut": "Enter",
                    "default": true,
                    "action": { "type": "copy_text", "text": "3" }
                }]
            }
        });
        let panel = parse_panel(&v).unwrap();
        assert_eq!(panel.blocks.len(), 2);
        assert_eq!(
            panel.default_native_action(),
            Some(NativeAction::CopyText("3".into()))
        );
    }

    #[test]
    fn rejects_html_like_block() {
        let v = json!({
            "blocks": [{ "type": "html", "html": "<b>x</b>" }]
        });
        assert!(parse_panel(&v).is_err());
    }

    #[test]
    fn notice_levels() {
        let v = json!({
            "blocks": [{ "type": "notice", "level": "error", "text": "表达式不完整" }]
        });
        let p = parse_panel(&v).unwrap();
        assert!(matches!(
            &p.blocks[0],
            PanelBlock::Notice { level, .. } if level == "error"
        ));
    }
}
