//! 全局快捷键：解析与展示。注册/改键由 ui 模块的热键线程用原生
//! RegisterHotKey（线程绑定 + 消息泵）完成；此处只做纯解析，便于测试复用。

pub const DEFAULT_HOTKEY: &str = "Alt+Space";

/// RegisterHotKey 修饰位。
pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;
/// 系统不重复触发（按住不松只触发一次）。
pub const MOD_NOREPEAT: u32 = 0x4000;

/// 解析 `Ctrl+Alt+K` → (mods(含 MOD_NOREPEAT), vk)。
/// 支持 Ctrl/Alt/Shift/Win + Space/Esc/Tab/A-Z/0-9/F1-F12。
pub fn parse_raw(s: &str) -> Option<(u32, u32)> {
    let mut mods = 0u32;
    let mut vk: Option<u32> = None;
    for part in s.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "alt" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" | "meta" => mods |= MOD_WIN,
            "space" => vk = Some(0x20),
            "esc" | "escape" => vk = Some(0x1B),
            "tab" => vk = Some(0x09),
            other => {
                if other.len() == 1 {
                    let c = other.chars().next()?;
                    if c.is_ascii_alphabetic() {
                        vk = Some(c.to_ascii_uppercase() as u32);
                    } else if c.is_ascii_digit() {
                        vk = Some(c as u32);
                    }
                } else if let Some(rest) = other.strip_prefix('f') {
                    let n: u32 = rest.parse().ok()?;
                    if (1..=12).contains(&n) {
                        vk = Some(0x70 + n - 1);
                    }
                }
            }
        }
    }
    Some((mods | MOD_NOREPEAT, vk?))
}

/// 人类可读标签（"ctrl+alt+k" → "Ctrl+Alt+K"）。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alt_space() {
        assert_eq!(parse_raw("Alt+Space"), Some((MOD_ALT | MOD_NOREPEAT, 0x20)));
    }

    #[test]
    fn parse_ctrl_shift_k() {
        let (mods, vk) = parse_raw("ctrl+shift+k").expect("parse");
        assert_eq!(mods, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT);
        assert_eq!(vk, 'K' as u32);
    }

    #[test]
    fn parse_f12_and_digit() {
        assert_eq!(parse_raw("Ctrl+F12").map(|(_, vk)| vk), Some(0x70 + 11));
        assert_eq!(parse_raw("Alt+1").map(|(_, vk)| vk), Some('1' as u32));
    }

    #[test]
    fn parse_rejects_empty_and_unknown() {
        assert!(parse_raw("").is_none());
        assert!(parse_raw("Alt+").is_none());
        assert!(parse_raw("Ctrl+Unknown").is_none());
        assert!(parse_raw("Space").is_some(), "单键也允许");
    }

    #[test]
    fn display_normalizes() {
        assert_eq!(display_label("ctrl+alt+k"), "Ctrl+Alt+K");
        assert_eq!(display_label("Alt+Space"), "Alt+Space");
    }
}
