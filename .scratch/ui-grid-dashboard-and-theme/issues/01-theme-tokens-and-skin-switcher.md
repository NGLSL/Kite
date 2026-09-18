# 01: Theme Tokens and Skin Switcher

**What to build:** 为 Kite 建立统一的语义化设计系统（Design System）与三档皮肤切换底座（深色 / 浅色 / 跟随系统）。深色模式严格采用 `#0F1115` 纯净深空碳黑底色搭配 `#2A2D32` 极细微光边框与 `#181A20` 悬浮层，浅色模式采用纯净通透象牙白 `#FFFFFF` 与柔和细边框，拒绝任何灰蒙蒙发虚的劣质磨砂。系统模式自动通过 Windows 注册表探测当前操作系统的深浅色模式。用户可在设置页自由切换外观模式，偏好持久化保存在本地 SQLite 中，重启后保持。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] 在 `src/ui/theme.rs` 中定义 `ThemeMode`（`Dark`, `Light`, `System`）与 `ThemeTokens`，包含窗口底色、微光边框、卡片底色、主副文字色阶与键帽样式。
- [x] 实现 Windows 注册表 `AppsUseLightTheme` 探测函数，安全回退到深色模式，无多余外部重量级依赖。
- [x] SQLite `Settings` 表支持 `theme_mode` 字段迁移与读写往返。
- [x] 在 `src/ui/settings/general.rs` 中提供外观模式分段切换器（`跟随系统` | `浅色` | `深色`），切换即时生效。
- [x] 补充主题与设置持久化的单元测试，确保 `cargo test` 全部通过。
