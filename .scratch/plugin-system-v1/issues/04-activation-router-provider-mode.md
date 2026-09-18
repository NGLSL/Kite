# 04: Activation Router + Provider Mode

**What to build:** 用户输入带 Trigger 的 Query（如 `=100`、`tr hello`）时，Kite 进入该 Provider 的 Provider Mode：搜索框保留，当前 Query 主要交给对应插件能力，不与 Core 应用/文件/其他插件混排评分。Trigger 不再匹配、Esc、或隐藏窗口时退出，回到 Core Search。Keyword 不得误触发（`tr` 不命中 `tree`）。V1 无 Global Provider：普通搜索永远优先 Core Search。

**Blocked by:** 03 — Manifest Registry + Command 静态入口

**Status:** resolved

- [x] prefix Trigger 命中：得到 plugin_id / provider_id / effective_query，并进入 Provider Mode
- [x] keyword Trigger 仅在「keyword 或 keyword+空白」边界匹配；`tree` 不触发 `tr`
- [x] Command 的 enter_provider 可进入对应 Provider Mode
- [x] 退出：Esc、Trigger 消失、切换 Provider、隐藏 Kite
- [x] Provider Mode 下不进行 Core Apps/Everything/其他插件的混排竞争
- [x] 无 Trigger 时 Activation Router 对 Core Search 可忽略（热路径仅极轻量判断）
- [x] 路由缝单测覆盖命中/边界/退出/无 Global；UI 状态测试覆盖进入与退出 Provider Mode
- [x] `cargo test` 全绿

## Comments

- `route_query` 纯函数；`keyword_requires_boundary` / `provider_mode_enters_on_trigger_and_esc_exits`。
- Esc 先退出 Provider Mode，不直接隐藏启动器。

## Notes

- 对应规格 Stage 2；本票允许 Provider Mode 先无真实插件数据（Host 未就绪时表现可为 pending/空），但模式切换与路由必须正确。
- 测试主缝：路由缝（纯函数）+ 最小 UI 状态缝。
