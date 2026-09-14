# 04: Alt+数字键启动失效

**What to build:** 搜索结果列表上按 Alt+1..9 能启动对应行。Windows 上 Alt 会改写逻辑键，须用物理键位（Digit/Numpad）识别。

**Blocked by:** None

**Status:** resolved

## Acceptance

- [x] 事件链路保留 physical_key
- [x] Alt+1..9 逻辑键 + 物理键位均可启动
- [x] 兼容小键盘 Numpad1..9
- [x] `cargo check --lib` 通过

## Comments

- 根因：`Message::KeyPressed` 只传了逻辑 `Key`；Windows + Alt 下 `key` 常不是 `Character("1")`，旧分支直接丢弃。
- 修复：`ui/mod.rs` 传入 `Physical`，`alt_digit_index` 优先字符、再回退 `Code::Digit1..9` / `Numpad1..9`。
- 需重新编译运行本机 UI 验证；单测无法覆盖真实 WM_SYSKEYDOWN。
