Status: resolved
Type: task

## Goal

修复现有搜索设置、扫描、缓存和历史管理体验，使设置值真正影响运行行为。

## Scope

- 让结果数量设置控制首屏和滚动加载上限。
- 修复一次点击触发两次重扫的问题。
- 新增或删除用户 Alias 后立即清理搜索缓存。
- 持久化“搜文件”选择，默认仍保持关闭。
- 增加清空历史和暂停记录历史的设置与 IPC。

## Acceptance Criteria

- 修改结果数量后，当前搜索实际显示数量受设置限制。
- 设置页点击一次重新扫描只执行一次扫描。
- Alias 变化后当前 Query 能立即反映结果变化。
- 重启 Kite 后，搜文件开关沿用上次选择。
- 清空历史后，空 Query 不再显示旧的最近使用记录。
- 暂停记录后，启动次数和 Query History 不再新增。
- 现有搜索行为和历史排序仍保持兼容。

## Validation

- `cd src-tauri; cargo test`
- `npm run build`
- Windows 实机检查一次重扫日志、设置生效、Alias 即时刷新和历史开关。

## Dependencies

无。完成后再交给 issue 02，避免共享 IPC 文件并行冲突。

## Out of Scope

- 结果右键菜单。
- Alias 目标选择器。
- 收藏排序策略。

## Comments

2026-09-13 实现记录：

- 结果数量设置生效：`search_apps` 在 IPC 边界把 limit 钳制到设置值（5–30）；
  前端首屏条数与滚动加载共用同一上限，设置变化后立即重搜。
- 双重重扫修复：`SettingsPanel` 自己只调用一次 `rescan_apps`，删除了
  SearchPanel 传入的 `onRescanned` 二次调用（index-ready 事件已负责刷新列表）。
- Alias 增删后 `set_user_alias` / `remove_user_alias` 现在清空搜索缓存，
  关闭设置页时前端还会对当前 Query 重搜一次。
- 搜文件选择持久化：Settings 新增 `search_files`（默认关）；
  搜索面板的「文件」开关通过 `save_settings` 落库，重启沿用。
- 历史控制：Settings 新增 `history_recording`（默认开），
  `HistoryDb::record_launch` 在暂停时整体跳过；新增 `clear_history` IPC
  （清 usage_history + query_history，保留固定项）并同步清搜索缓存。
- 设置页「通用」新增「记录使用历史」开关与「清空使用历史」按钮。
- 验证：`cargo test` 127 通过；`npm run build` 通过。
  待人工实机确认：一次重扫日志、设置即时生效、重启后搜文件开关保持。

2026-09-13 追加（用户决定）：「结果数量」设置整个移除——设置页不再展示，
Settings.max_results 字段、IPC 参数与搜索侧钳制一并删除；首屏条数恢复为
按窗口高度自适应（6–30），滚动加载上限回到 Rust MAX_RESULTS(200)。
数据库里遗留的 max_results 键为无害孤儿数据，不清理。
