# 05: 控制面板图标缺失 + Alt+数字变成输入数字

**What to build:** 系统 shell 入口（控制面板等）在后台扫描线程能提出图标；Alt+1..9 在不同 Windows/IME/键盘布局下都能启动对应行，且不会把数字写进搜索框。

**Blocked by:** 04

**Status:** resolved

## Acceptance

- [x] shell 图标提取前 CoInitializeEx（后台线程 COM）
- [x] Alt 状态本地跟踪 + 物理 Digit/Numpad + modifiers.alt() 三路兼容
- [x] 按住 Alt 时丢弃 text_input 误插入的纯数字 Query
- [x] 启动前回滚 query_at_alt
- [x] `cargo test --lib` 全绿

## Comments

- 图标根因：`request_build` 后台线程调 Shell API 未 COM 初始化，控制面板等虚拟命名空间提取失败 → UI 字母兜底。
- Alt 根因：Windows SYSKEY 下 `modifiers.alt()` 可能为假，但 text_input 仍插入 `text="1"`；旧逻辑既认不出快捷键又拦不住插入。
- 兼容策略：Named::Alt 按下/抬起跟踪 `alt_down`；`alt_digit_index` 接受 mods.alt 或 alt_down；物理键位覆盖主键盘与小键盘。
