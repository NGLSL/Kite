//! 官方 DevTools：单 Runtime 多 Provider（uuid / hash / ts / json）。

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

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
            let text = query.trim();
            if text.is_empty() {
                return kite_plugin_sdk::panel_response(json!({
                    "blocks": [{ "type": "notice", "level": "info", "text": "输入待哈希文本（hash …）" }],
                    "actions": []
                }));
            }
            // 轻量演示：非加密用途的 FNV-1a 64，避免官方样例再拉 sha2 依赖
            let h = fnv1a64(text.as_bytes());
            let hex = format!("{h:016x}");
            kite_plugin_sdk::panel_response(json!({
                "blocks": [{
                    "type": "key_value",
                    "items": [{ "key": "FNV-1a-64", "value": hex }]
                }],
                "actions": [{
                    "id": "copy",
                    "label": "复制",
                    "default": true,
                    "action": { "type": "copy_text", "text": hex }
                }]
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
                    "blocks": [{ "type": "notice", "level": "info", "text": "独立面板：设置 → 插件 → 打开 JSON 面板。也可 json {...} 快速格式化。" }],
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

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
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
