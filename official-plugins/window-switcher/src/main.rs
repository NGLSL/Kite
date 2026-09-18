//! 官方 Window Switcher：keyword `win`，List；Enter → plugin/execute activate_window。

use serde_json::{json, Value};

fn main() {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = kite_plugin_sdk::serve_loop(
        &mut stdout,
        |_provider, query| list_result(query),
        |action_id, payload| {
            if action_id == "activate_window" {
                Ok(activate_window(payload))
            } else {
                Err(format!("unknown action {action_id}"))
            }
        },
    );
}

fn list_result(query: &str) -> Value {
    let q = query.trim().to_lowercase();
    let mut items = Vec::new();
    for (hwnd, title) in enum_windows_or_mock() {
        let title_l = title.to_lowercase();
        if !q.is_empty() && !title_l.contains(&q) {
            continue;
        }
        items.push(json!({
            "id": format!("window-{hwnd}"),
            "title": title,
            "subtitle": format!("hwnd={hwnd}"),
            "priority": 10,
            "action": {
                "type": "plugin_action",
                "action_id": "activate_window",
                "payload": { "hwnd": hwnd }
            }
        }));
    }
    kite_plugin_sdk::list_response(Value::Array(items))
}

fn enum_windows_or_mock() -> Vec<(i64, String)> {
    let w = enum_windows();
    if w.is_empty() {
        vec![
            (1, "Visual Studio Code — Kite".into()),
            (2, "IntelliJ IDEA — Kite".into()),
            (3, "Explorer".into()),
        ]
    } else {
        w
    }
}

fn enum_windows() -> Vec<(i64, String)> {
    #[cfg(windows)]
    {
        windows_enum::enum_visible()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn activate_window(payload: &Value) -> Value {
    let Some(hwnd) = payload.get("hwnd").and_then(|v| v.as_i64()) else {
        return json!({"ok": false, "reason": "missing hwnd"});
    };
    #[cfg(windows)]
    {
        if windows_enum::activate(hwnd) {
            return json!({"ok": true, "hwnd": hwnd});
        }
        return json!({"ok": false, "reason": "activate failed"});
    }
    #[cfg(not(windows))]
    {
        json!({"ok": false, "reason": "no win32 window", "hwnd": hwnd})
    }
}

#[cfg(windows)]
mod windows_enum {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    pub fn enum_visible() -> Vec<(i64, String)> {
        unsafe {
            let mut out: Vec<(i64, String)> = Vec::new();
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut out as *mut _ as isize));
            out
        }
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<(i64, String)>);
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return TRUE;
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return TRUE;
        }
        let title = String::from_utf16_lossy(&buf[..n as usize]).trim().to_string();
        if title.is_empty() {
            return TRUE;
        }
        out.push((hwnd.0 as i64, title));
        TRUE
    }

    pub fn activate(hwnd: i64) -> bool {
        let hwnd = HWND(hwnd as *mut _);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd).as_bool()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_has_plugin_action() {
        let v = list_result("kite");
        assert_eq!(v["type"], "list");
        let items = v["items"].as_array().unwrap();
        assert!(!items.is_empty());
        assert_eq!(items[0]["action"]["type"], "plugin_action");
        assert_eq!(items[0]["action"]["action_id"], "activate_window");
    }
}
