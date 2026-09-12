//! 全局快捷键：解析「Ctrl+Alt+K」类字符串并注册/切换。

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

/// 默认快捷键。
pub const DEFAULT_HOTKEY: &str = "Alt+Space";

/// 将 `Ctrl+Alt+K` / `Alt+Space` 解析为 Shortcut。
pub fn parse_hotkey(s: &str) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for raw in s.split('+') {
        let part = raw.trim();
        if part.is_empty() {
            return None;
        }
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "alt" => mods |= Modifiers::ALT,
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "super" | "meta" | "win" | "cmd" => mods |= Modifiers::SUPER,
            "space" => code = Some(Code::Space),
            "escape" | "esc" => code = Some(Code::Escape),
            "enter" | "return" => code = Some(Code::Enter),
            "tab" => code = Some(Code::Tab),
            "backspace" => code = Some(Code::Backspace),
            "delete" | "del" => code = Some(Code::Delete),
            "insert" => code = Some(Code::Insert),
            "home" => code = Some(Code::Home),
            "end" => code = Some(Code::End),
            "pageup" => code = Some(Code::PageUp),
            "pagedown" => code = Some(Code::PageDown),
            "up" => code = Some(Code::ArrowUp),
            "down" => code = Some(Code::ArrowDown),
            "left" => code = Some(Code::ArrowLeft),
            "right" => code = Some(Code::ArrowRight),
            _ => {
                code = parse_key_code(&lower);
            }
        }
    }
    Some(Shortcut::new(if mods.is_empty() { None } else { Some(mods) }, code?))
}

fn parse_key_code(lower: &str) -> Option<Code> {
    if let Some(rest) = lower.strip_prefix('f') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            let n: u8 = rest.parse().ok()?;
            return match n {
                1 => Some(Code::F1),
                2 => Some(Code::F2),
                3 => Some(Code::F3),
                4 => Some(Code::F4),
                5 => Some(Code::F5),
                6 => Some(Code::F6),
                7 => Some(Code::F7),
                8 => Some(Code::F8),
                9 => Some(Code::F9),
                10 => Some(Code::F10),
                11 => Some(Code::F11),
                12 => Some(Code::F12),
                _ => None,
            };
        }
    }
    if lower.len() == 1 {
        let c = lower.chars().next()?;
        if c.is_ascii_digit() {
            return match c {
                '0' => Some(Code::Digit0),
                '1' => Some(Code::Digit1),
                '2' => Some(Code::Digit2),
                '3' => Some(Code::Digit3),
                '4' => Some(Code::Digit4),
                '5' => Some(Code::Digit5),
                '6' => Some(Code::Digit6),
                '7' => Some(Code::Digit7),
                '8' => Some(Code::Digit8),
                '9' => Some(Code::Digit9),
                _ => None,
            };
        }
        if c.is_ascii_alphabetic() {
            return Some(match c {
                'a' => Code::KeyA,
                'b' => Code::KeyB,
                'c' => Code::KeyC,
                'd' => Code::KeyD,
                'e' => Code::KeyE,
                'f' => Code::KeyF,
                'g' => Code::KeyG,
                'h' => Code::KeyH,
                'i' => Code::KeyI,
                'j' => Code::KeyJ,
                'k' => Code::KeyK,
                'l' => Code::KeyL,
                'm' => Code::KeyM,
                'n' => Code::KeyN,
                'o' => Code::KeyO,
                'p' => Code::KeyP,
                'q' => Code::KeyQ,
                'r' => Code::KeyR,
                's' => Code::KeyS,
                't' => Code::KeyT,
                'u' => Code::KeyU,
                'v' => Code::KeyV,
                'w' => Code::KeyW,
                'x' => Code::KeyX,
                'y' => Code::KeyY,
                'z' => Code::KeyZ,
                _ => return None,
            });
        }
    }
    None
}

/// 人类可读标签。
pub fn display_label(hotkey: &str) -> String {
    hotkey
        .split('+')
        .map(|p| {
            let t = p.trim();
            match t.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => "Ctrl".into(),
                "alt" => "Alt".into(),
                "shift" => "Shift".into(),
                "super" | "meta" | "win" | "cmd" => "Win".into(),
                "space" => "Space".into(),
                _ => {
                    if t.len() == 1 {
                        t.to_ascii_uppercase()
                    } else {
                        t.to_string()
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// 注销全部后注册新快捷键。
pub fn reregister(app: &AppHandle, hotkey: &str) -> Result<String, String> {
    let shortcut = parse_hotkey(hotkey).ok_or_else(|| format!("无法解析快捷键: {hotkey}"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(shortcut).map_err(|e| format!("注册失败: {e}"))?;
    crate::log::info(&format!("hotkey registered: {}", display_label(hotkey)));
    Ok(display_label(hotkey))
}

/// 从历史库读快捷键；无则默认。
pub fn load_hotkey(app: &AppHandle) -> String {
    app.try_state::<crate::state::AppState>()
        .and_then(|s| {
            s.history
                .lock()
                .ok()
                .and_then(|h| h.get_setting("hotkey").ok())
        })
        .filter(|s| !s.is_empty() && parse_hotkey(s).is_some())
        .unwrap_or_else(|| DEFAULT_HOTKEY.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alt_space() {
        assert!(parse_hotkey("Alt+Space").is_some());
    }

    #[test]
    fn parse_ctrl_shift_k() {
        assert!(parse_hotkey("Ctrl+Shift+K").is_some());
        assert!(parse_hotkey("ctrl+alt+k").is_some());
    }

    #[test]
    fn parse_f12() {
        assert!(parse_hotkey("Ctrl+F12").is_some());
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("Alt+").is_none());
        assert!(parse_hotkey("Ctrl+Unknown").is_none());
    }

    #[test]
    fn display_normalizes() {
        assert_eq!(display_label("ctrl+alt+k"), "Ctrl+Alt+K");
        assert_eq!(display_label("Alt+Space"), "Alt+Space");
    }
}
