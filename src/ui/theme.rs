//! Kite 统一设计体系与主题系统（Theme System & Design Tokens）。
//!
//! 支持三档模式：
//! - Dark（深色模式）：#0F1115 纯净深空碳黑底色 + #2A2D32 极细微光边框 + #181A20 悬浮卡片 + #E5E7EB 纯正高亮文字；
//! - Light（浅色模式）：#FFFFFF 纯净象牙白 + #E5E7EB 柔和细边框 + #F8F9FA 卡片底色 + #111827 锐利深灰文字；
//! - System（跟随系统）：读取 Windows 注册表 `AppsUseLightTheme` 自动识别系统深浅色。

use iced::color;
use iced::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemeMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "light" => ThemeMode::Light,
            "system" => ThemeMode::System,
            _ => ThemeMode::Dark,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
            ThemeMode::System => "system",
        }
    }

    pub fn is_dark(&self) -> bool {
        match self {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => crate::system::theme::is_windows_dark_mode(),
        }
    }

    pub fn tokens(&self) -> ThemeTokens {
        if self.is_dark() {
            ThemeTokens::dark()
        } else {
            ThemeTokens::light()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeTokens {
    pub is_dark: bool,
    /// 窗口主底色：深色 #0F1115，浅色 #FFFFFF
    pub bg_window: Color,
    /// 浮层/卡片底色：深色 #181A20，浅色 #F8F9FA
    pub bg_elevated: Color,
    /// 搜索行/输入框底色：深色 #14161C，浅色 #F3F4F6
    pub bg_input: Color,
    /// 外窗 1px 细边框：深色 #2A2D32，浅色 #E5E7EB
    pub border_window: Color,
    /// 卡片/分割 1px 极细边框：深色 rgba(255,255,255,0.06)，浅色 rgba(0,0,0,0.06)
    pub border_subtle: Color,
    /// 主文字颜色：深色 #E5E7EB，浅色 #111827
    pub text_primary: Color,
    /// 次级文字/路径颜色：深色 #9CA3AF，浅色 #6B7280
    pub text_muted: Color,
    /// 强调色（蓝）：深色 #3B82F6，浅色 #2563EB
    pub accent: Color,
    /// 激活/选中行底色：深色 rgba(255,255,255,0.08)，浅色 #EFF6FF
    pub active_bg: Color,
    /// 激活/选中边框：深色 rgba(59,130,246,0.30)，浅色 rgba(37,99,235,0.25)
    pub active_border: Color,
    /// 物理键帽底色：深色 #1E222A，浅色 #FFFFFF
    pub keycap_bg: Color,
    /// 物理键帽边框：深色 rgba(255,255,255,0.12)，浅色 #E2E8F0
    pub keycap_border: Color,
    /// 物理键帽文字：深色 #D1D5DB，浅色 #4B5563
    pub keycap_text: Color,
}

impl ThemeTokens {
    pub fn dark() -> Self {
        Self {
            is_dark: true,
            bg_window: color!(0x0F_11_15),
            bg_elevated: color!(0x18_1A_20),
            bg_input: color!(0x14_16_1C),
            border_window: color!(0x2A_2D_32),
            border_subtle: Color::from_rgba(1.0, 1.0, 1.0, 0.06),
            text_primary: color!(0xE5_E7_EB),
            text_muted: color!(0x9C_A3_AF),
            accent: color!(0x3B_82_F6),
            active_bg: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
            active_border: Color::from_rgba(0.23, 0.51, 0.96, 0.30),
            keycap_bg: color!(0x1E_22_2A),
            keycap_border: Color::from_rgba(1.0, 1.0, 1.0, 0.12),
            keycap_text: color!(0xD1_D5_DB),
        }
    }

    pub fn light() -> Self {
        Self {
            is_dark: false,
            bg_window: color!(0xFF_FF_FF),
            bg_elevated: color!(0xF8_F9_FA),
            bg_input: color!(0xF3_F4_F6),
            border_window: color!(0xE5_E7_EB),
            border_subtle: Color::from_rgba(0.0, 0.0, 0.0, 0.06),
            text_primary: color!(0x11_18_27),
            text_muted: color!(0x6B_72_80),
            accent: color!(0x25_63_EB),
            active_bg: color!(0xEF_F6_FF),
            active_border: Color::from_rgba(0.15, 0.39, 0.92, 0.25),
            keycap_bg: color!(0xFF_FF_FF),
            keycap_border: color!(0xE2_E8_F0),
            keycap_text: color!(0x4B_55_63),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_mode_parse_and_serialize() {
        assert_eq!(ThemeMode::parse("dark"), ThemeMode::Dark);
        assert_eq!(ThemeMode::parse("DARK"), ThemeMode::Dark);
        assert_eq!(ThemeMode::parse("light"), ThemeMode::Light);
        assert_eq!(ThemeMode::parse("system"), ThemeMode::System);
        assert_eq!(ThemeMode::parse("other"), ThemeMode::Dark);

        assert_eq!(ThemeMode::Dark.as_str(), "dark");
        assert_eq!(ThemeMode::Light.as_str(), "light");
        assert_eq!(ThemeMode::System.as_str(), "system");
    }

    #[test]
    fn theme_tokens_contrast_and_distinction() {
        let dark = ThemeTokens::dark();
        let light = ThemeTokens::light();

        assert!(dark.is_dark);
        assert!(!light.is_dark);

        // 深色底深，浅色底浅
        assert!(dark.bg_window.r < 0.2 && dark.bg_window.g < 0.2 && dark.bg_window.b < 0.2);
        assert!(light.bg_window.r > 0.9 && light.bg_window.g > 0.9 && light.bg_window.b > 0.9);

        // 文字高对比度
        assert!(dark.text_primary.r > 0.8);
        assert!(light.text_primary.r < 0.2);
    }
}
