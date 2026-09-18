# 04: Plugin Instant Card and Dynamic Action Bar

**What to build:** 升级插件激活与 Provider 模式下的视觉呈现与交互流转。命中插件（如计算器 `=1+1`、时间戳 `ts`、文件搜索 `f`）时，搜索框内部内嵌精致的模式胶囊标签（如 `[🧮 计算器]`），用户按 Backspace 即可退回全部搜索；移除搜索框右侧突兀的 `[文件]` 按钮。计算器等 Provider 模式展现全宽大字号（36px）即时结果大卡片及进制转换表格。底部 Action Bar 升级为动态微立体实体键帽组合，根据当前网格态、双列搜索态、插件态自动呈现当前上下文的操作说明。

**Blocked by:** 03: Search Two-Column Cards

**Status:** resolved

- [x] 搜索框左侧支持内嵌模式胶囊标签（Mode Tag Chip），按 Backspace 退出模式；移除右侧孤立的 `[文件]` 按钮。
- [x] 优化计算器等 Provider 模式的 Panel 渲染，使用 36px 醒目等宽字号大卡片，搭配进制换算或格式化子卡片。
- [x] 底部 Action Bar 随模式动态切换：
  - 网格态：`↑↓←→ 选择` · `↵ 打开` · `Esc 隐藏`
  - 搜索态：`↵ 打开应用` · `Ctrl+↵ 所在目录` · `Esc 关闭`
  - 插件态：`↵ 复制结果` · `Ctrl+↵ 复制并继续` · `⌫ 退出模式`
- [x] 底部快捷键全部改用实体机械键帽（Keycap）微立体样式呈现。
- [x] 运行全套自动化测试 `cargo test` 与编译检查 `cargo check`，进行多场景端到端验证。
