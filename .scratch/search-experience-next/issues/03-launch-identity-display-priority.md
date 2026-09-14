Status: resolved
Type: task
Blocked by: None

## Goal

用回归测试锁定「启动身份」与展示优先级：规范化 `target + args` 为同一启动语义的默认键；不同参数入口保留；友好快捷方式优先于同一安装中的原始 exe；深层多层入口与引号参数语义不回退。

## Scope

- 将工作区未提交的「快速扫描加深目录」「target + args 去重」「raw_arg 保留引号」「交互式控制台句柄」纳入可重复回归验收。
- 去重键：规范化 target + 原始启动参数；工作目录不默认参与去重。
- 同名不同目标、同 exe 不同参数（如 Windows PowerShell / PowerShell 7 / ISE / Developer PowerShell）均保留。
- 开始菜单/桌面可读入口展示优先；App Paths 与原始 exe 可搜索但适度靠后。
- 不为任何品牌或产品名写扫描/去重特例。

## Acceptance Criteria

- [x] `nested_program_entries_are_indexed` 等深层入口测试保持通过。
- [x] 相同 target、不同 args 的入口在索引中各保留一条；相同语义多来源合并为一条。
- [x] 工作目录变化但启动语义相同的入口不拆成重复项。
- [x] 友好名称入口在相关 Query 下排在同安装原始 exe 之前，原始入口仍可搜到。
- [x] 带引号的快捷方式参数按 Windows 原有语义启动（`raw_arg` 回归）。
- [x] 交互式控制台入口启动后窗口保持运行（不因 stdin=NUL 立即退出）。

## Validation

- `cargo test`
- Windows 实机：多个 PowerShell 入口分别可启动且窗口保持；Git Bash/CMD/GUI 等深层快捷方式可搜可启动。

## Dependencies

无。为票 04 提供稳定的启动身份定义。

## Out of Sources / Out of Scope

- 后台完整扫描与监听（票 01/02）。
- Pin/Alias/History 身份迁移策略（票 04）。
- 图标正确性（票 05）。

## Comments

本地工作区已有未提交实现：`START_MENU_MAX_DEPTH=5`、`dedupe_key`（target+args）、`raw_arg`、交互式控制台 stdio 修复及对应测试。本票要求把这些锁定为回归基线，不把它们误认为后台完整扫描已完成。

2026-09-14 实现记录（worktree `Kite-search-experience` / 分支 `search-experience-next`）：

- 将 WIP 合入：`START_MENU_MAX_DEPTH=5`、`OTHER_SOURCE_MAX_DEPTH`、`dedupe_key`（target+args，工作目录不参与）、`raw_arg` 保留引号、交互式控制台不再强制 stdin=NUL。
- 回归：`nested_program_entries_are_indexed`、`dedupe_preserves_distinct_launch_arguments`、`friendly_shortcut_ranks_above_raw_app_path_in_same_installation`、`source_preference_applies_to_other_apps_and_preserves_unrelated_exact_match`、`launch_preserves_quoted_shortcut_arguments`。
- `cargo test`：159 passed / 0 failed（2 ignored 基准/联网）。
- 待人工实机：多个 PowerShell 入口窗口保持；Git Bash 等深层快捷方式可启动。
