//! 官方 DevTools：单 Runtime 多 Provider（uuid / hash / ts / json / base64）。

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn main() {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = kite_plugin_sdk::serve_loop(
        &mut stdout,
        |provider, query| provider_panel(provider, query),
        |_action_id, _payload| Ok(json!({"ok": true})),
    );
}

fn provider_panel(provider: &str, query: &str) -> Value {
    match provider {
        "uuid" => {
            let id = simple_uuid();
            kite_plugin_sdk::panel_response(json!({
                "blocks": [{
                    "type": "value",
                    "label": "UUID",
                    "value": id,
                    "selectable": true
                }],
                "actions": [{
                    "id": "copy",
                    "label": "复制",
                    "default": true,
                    "action": { "type": "copy_text", "text": id }
                }]
            }))
        }
        "hash" => {
            let text = query;
            if text.is_empty() {
                return kite_plugin_sdk::panel_response(json!({
                    "blocks": [{ "type": "notice", "level": "info", "text": "输入待哈希文本（hash …）" }],
                    "actions": []
                }));
            }
            let hex = format!("{:x}", Sha256::digest(text.as_bytes()));
            kite_plugin_sdk::panel_response(json!({
                "blocks": [{
                    "type": "key_value",
                    "items": [{ "key": "SHA-256", "value": hex }]
                }],
                "actions": [{
                    "id": "copy",
                    "label": "复制",
                    "default": true,
                    "action": { "type": "copy_text", "text": hex }
                }]
            }))
        }
        "base64" => {
            let raw = query.trim();
            let text = if raw.is_empty() {
                "宿主 Base64 工具：搜索 base64 后 Enter 打开独立窗，可在文本与 Base64 之间编码或解码。"
                    .to_string()
            } else {
                format!(
                    "宿主 Base64 工具：搜索 base64 后 Enter 打开独立窗，已预填 {} 个字符。",
                    raw.chars().count()
                )
            };
            kite_plugin_sdk::panel_response(json!({
                "blocks": [{ "type": "notice", "level": "info", "text": text }],
                "actions": []
            }))
        }
        "ts" => {
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let text = ms.to_string();
            kite_plugin_sdk::panel_response(json!({
                "blocks": [
                    { "type": "text", "text": "Unix 毫秒时间戳", "style": "secondary" },
                    { "type": "value", "value": text, "selectable": true }
                ],
                "actions": [{
                    "id": "copy",
                    "label": "复制",
                    "default": true,
                    "action": { "type": "copy_text", "text": text }
                }]
            }))
        }
        "json" => {
            let raw = query.trim();
            if raw.is_empty() {
                return kite_plugin_sdk::panel_response(json!({
                    "blocks": [{ "type": "notice", "level": "info", "text": "宿主 JSON 工具：搜索 json 后 Enter 打开独立窗；也可在设置 → 插件 → 开发者工具说明里点「打开 JSON 工具」。" }],
                    "actions": []
                }));
            }
            match serde_json::from_str::<Value>(raw) {
                Ok(v) => {
                    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.into());
                    kite_plugin_sdk::panel_response(json!({
                        "blocks": [
                            { "type": "text", "text": "格式化 JSON", "style": "secondary" },
                            { "type": "value", "value": pretty, "selectable": true }
                        ],
                        "actions": [{
                            "id": "copy",
                            "label": "复制",
                            "default": true,
                            "action": { "type": "copy_text", "text": pretty }
                        }]
                    }))
                }
                Err(e) => kite_plugin_sdk::panel_response(json!({
                    "blocks": [{ "type": "notice", "level": "error", "text": format!("JSON 无效: {e}") }],
                    "actions": []
                })),
            }
        }
        _ => kite_plugin_sdk::empty_response(),
    }
}

/// 简易 UUIDv4 形态（基于时间+计数的伪随机，演示用）。
fn simple_uuid() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let a = t ^ 0x9e3779b97f4a7c15;
    let b = t.rotate_left(17) ^ 0x517cc1b727220a95;
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        a as u32,
        (a >> 32) as u16,
        ((a >> 48) as u16) & 0x0fff,
        ((b >> 4) as u16) & 0x0fff,
        b & 0xffff_ffff_ffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_provider_shapes() {
        let uuid = provider_panel("uuid", "");
        assert_eq!(uuid["type"], "panel");
        assert_eq!(uuid["panel"]["blocks"][0]["type"], "value");

        let hash = provider_panel("hash", "abc");
        assert_eq!(hash["panel"]["blocks"][0]["type"], "key_value");
        assert_eq!(hash["panel"]["blocks"][0]["items"][0]["key"], "SHA-256");
        assert_eq!(
            hash["panel"]["blocks"][0]["items"][0]["value"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let base64 = provider_panel("base64", "hello");
        assert_eq!(base64["type"], "panel");
        assert_eq!(base64["panel"]["blocks"][0]["type"], "notice");
        assert!(base64["panel"]["blocks"][0]["text"]
            .as_str()
            .unwrap()
            .contains("5 个字符"));

        let ts = provider_panel("ts", "");
        assert_eq!(ts["panel"]["blocks"][1]["type"], "value");

        let json_ok = provider_panel("json", r#"{"a":1}"#);
        assert!(json_ok["panel"]["blocks"][1]["value"]
            .as_str()
            .unwrap()
            .contains('\n'));

        assert_eq!(provider_panel("nope", "")["type"], "empty");
    }

    #[test]
    fn uuid_shape() {
        let id = simple_uuid();
        let parts: Vec<_> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[2].len(), 4);
        assert!(parts[2].starts_with('4'));
    }
}
