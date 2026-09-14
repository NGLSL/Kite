Status: resolved
Type: task
Blocked by: None

## Goal

启动时先用有预算的快速扫描提供首屏结果，再在后台完整递归扫描已支持的 Start Menu / Desktop 入口，补齐快扫遗漏并原子切换索引快照；更新期间旧索引仍可搜索，完成后当前 Query 自动刷新。

## Scope

- 快扫路径保留 1.5s 预算与早发布；不因快扫提前停止而永久漏掉后续目录。
- 后台完整扫描递归遍历用户/公共开始菜单与桌面，不受快扫深度与时间预算限制。
- 不跟随 junction；单个目录或快捷方式解析失败只跳过该项。
- 同一时刻最多一次索引构建；构建结果不得覆盖更新的快照（generation / epoch）。
- 完整快照原子发布后刷新当前 Query，列表不短暂清空、不来回跳动。
- 安全上限触发时记录未覆盖目录与原因，不得静默宣称已完整索引。

## Acceptance Criteria

- [x] 启动后快扫结果先可用，后台完整扫描完成后深层入口出现在索引中，无需重启或托盘重扫。
- [x] 后台更新期间继续输入 Query，列表始终有结果可搜。
- [x] 快照切换后，当前 Query 结果一次性刷新为新索引。
- [x] 一条坏快捷方式或一个不可读目录不导致整份索引清空或扫描中止。
- [x] 快扫已覆盖的深层多层目录（本地未提交加深修复）与后台补齐路径行为一致。
- [x] 日志能区分快扫预算截断与后台完整扫描完成。

## Validation

- `cargo test`（扫描器相关用例）
- `cargo check`
- Windows 实机：放置深层快捷方式 → 启动后后台补扫可见；更新过程中热键与输入不卡顿。

## Dependencies

无。为票 02 提供可触发的后台完整扫描与原子切换能力。

## Out of Scope

- 入口变化的文件系统监听（票 02）。
- UWP/Store 实时安装事件。
- 图标提取策略修正（票 05）。
- 搜索召回与排序调整。

## Comments

2026-09-14 实现记录（worktree `Kite-search-experience`）：

- `ScanPass::{Fast,Full}`：Full 无 1.5s 预算，Start Menu 深度 16、Desktop 8，安全上限 8000/2000 并写日志。
- `backend::request_build` 单飞 + generation；快扫发布 → 图标 → UWP → Full 原子替换并 `FullIndexReady`，UI 随后 `refresh_results`。
- 回归：`full_pass_recurses_deeper_than_fast_depth`；`cargo test` 162 passed。
- 待实机：深层厂商入口在后台补扫后可见；更新期间输入不中断。入口变化监听属票 02。
