# 05: Settings Theme Tokens and Surface Redesign

**What to build:** 将设置页（`src/ui/settings/`）全面接入 `ThemeTokens` 主题设计体系，消除写死白底（`BG_PANEL = #FFFFFF`）造成的严重黑白割裂。当用户在外观模式中切换【深色】/【浅色】时，设置窗口与主启动器实现实时的联动换肤变色。同步重塑设置页的实体表面质感：升级左侧导航栏、设置卡片（`section_card`）、Toggle 开关、输入框和按钮的视觉层级，与主启动器保持高度一致的高级原生桌面质感。

**Blocked by:** 01: Theme Tokens and Skin Switcher

**Status:** resolved

- [x] 在 `src/ui/settings/mod.rs` 中引入 `tokens: ThemeTokens = state.theme_tokens()`，替换写死的纯白常量 `BG_PANEL` / `BG_ELEVATED` / `BORDER` / `TEXT` / `WINDOW_BORDER`。
- [x] 改造左侧导航栏（Logo、标题栏、菜单选项 Hover / Active 态），在深色模式下呈现 `#14161C` 底色与微光高亮。
- [x] 改造 `src/ui/settings/widgets.rs` 中的共用组件：
  - `section_card`：采用深色表面 `#181A20`（深色）/ `#FFFFFF`（浅色），搭配 1px 微光边框；
  - `toggle` 开关：优化开关尺寸与高对比度着色；
  - 按钮体系（`primary_button`, `std_button`, `ghost_button`）与 `text_input` 接入 Token。
- [x] 改造各子面板（`general.rs`, `search_engine.rs`, `hotkey.rs`, `alias.rs`, `index.rs`, `plugins.rs`, `about.rs`）传递并消费 `ThemeTokens`。
- [x] 确保在通用设置中点击深色/浅色时，设置页自身即时重绘换肤，并保持所有已有测试 100% 通过。
