//! Kite 官方插件最小 SDK：Content-Length 帧 + JSON-RPC 2.0 over stdio。
//!
//! 宿主契约见 `docs/PLUGIN-DEVELOPMENT.md`。插件进程不链接 Kite 本体。

use std::io::{Read, Write};

use serde_json::{json, Value};

/// 读满 n 字节。
fn read_exact(stdin: &mut impl Read, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut off = 0;
    while off < n {
        match stdin.read(&mut buf[off..]) {
            Ok(0) | Err(_) => return None,
            Ok(k) => off += k,
        }
    }
    Some(buf)
}

/// 读一帧：`Content-Length: N\r\n\r\n{JSON}`。EOF 返回 None。
pub fn read_message(stdin: &mut impl Read) -> Option<Value> {
    let mut header = Vec::new();
    loop {
        let mut b = [0u8; 1];
        match stdin.read(&mut b) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        header.push(b[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
        if header.len() > 64 * 1024 {
            return None;
        }
    }
    let header_s = String::from_utf8_lossy(&header);
    let mut length = 0usize;
    for line in header_s.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    if length == 0 {
        return None;
    }
    let body = read_exact(stdin, length)?;
    serde_json::from_slice(&body).ok()
}

/// 写一帧到 stdout。
pub fn write_message(stdout: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(stdout, "Content-Length: {}\r\n\r\n", body.len())?;
    stdout.write_all(&body)?;
    stdout.flush()
}

pub fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

pub fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

/// 初始化应答：声明 plugin_api=1 与 capabilities。
pub fn initialize_result() -> Value {
    json!({
        "plugin_api": 1,
        "capabilities": { "query": true, "execute": true, "cancellation": false }
    })
}

/// 标准事件循环：initialize / query / execute。
/// `on_query(provider_id, query) -> result`，`on_execute(action_id, payload) -> result | Err(msg)`。
pub fn serve_loop<Q, E>(
    stdout: &mut impl Write,
    mut on_query: Q,
    mut on_execute: E,
) -> std::io::Result<()>
where
    Q: FnMut(&str, &str) -> Value,
    E: FnMut(&str, &Value) -> Result<Value, String>,
{
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    while let Some(msg) = read_message(&mut stdin) {
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "plugin/initialize" => {
                write_message(stdout, &rpc_result(id, initialize_result()))?;
            }
            "plugin/query" => {
                let provider = params
                    .get("provider_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                write_message(stdout, &rpc_result(id, on_query(provider, query)))?;
            }
            "plugin/execute" => {
                let action_id = params
                    .get("action_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let payload = params.get("payload").cloned().unwrap_or(Value::Null);
                let resp = match on_execute(action_id, &payload) {
                    Ok(v) => rpc_result(id, v),
                    Err(e) => rpc_error(id, -32601, &e),
                };
                write_message(stdout, &resp)?;
            }
            _ if id.is_null() => {}
            _ => {
                write_message(stdout, &rpc_error(id, -32601, "method not found"))?;
            }
        }
    }
    Ok(())
}

/// List 响应。
pub fn list_response(items: Value) -> Value {
    json!({ "type": "list", "items": items })
}

/// Panel 响应。
pub fn panel_response(panel: Value) -> Value {
    json!({ "type": "panel", "panel": panel })
}

/// Empty 响应。
pub fn empty_response() -> Value {
    json!({ "type": "empty" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        let msg = json!({"jsonrpc":"2.0","id":1,"result":{"type":"empty"}});
        write_message(&mut buf, &msg).unwrap();
        let mut cursor = buf.as_slice();
        let got = read_message(&mut cursor).unwrap();
        assert_eq!(got, msg);
        assert!(read_message(&mut cursor).is_none());
    }

    #[test]
    fn serve_loop_initialize_and_query() {
        let input = {
            let mut v = Vec::new();
            write_message(
                &mut v,
                &json!({"jsonrpc":"2.0","id":1,"method":"plugin/initialize"}),
            )
            .unwrap();
            write_message(
                &mut v,
                &json!({
                    "jsonrpc":"2.0","id":2,"method":"plugin/query",
                    "params":{"provider_id":"calculate","query":"1+2"}
                }),
            )
            .unwrap();
            v
        };
        // serve_loop 从 stdin 读，这里直接测 read/write 帧与 result 构造
        let mut cursor = input.as_slice();
        let m1 = read_message(&mut cursor).unwrap();
        assert_eq!(m1["method"], "plugin/initialize");
        let m2 = read_message(&mut cursor).unwrap();
        assert_eq!(m2["params"]["query"], "1+2");
        let _ = rpc_result(m2["id"].clone(), empty_response());
    }
}
