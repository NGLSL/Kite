# 03: App Paths/Uninstall 启发式过滤

**What to build:** App Paths 与 Uninstall 注册表不再「路径存在就变成应用行」。系统组件、helper、updater、installer、SDK/runtime 端点，以及指向 System32、WindowsApps、WinGet Links、Chocolatey bin 等目录的条目，不物化为独立 `AppItem`。过滤可单测，不依赖真实 HKLM 全量。

**Blocked by:** None (can start immediately)

**Status:** implemented

- [x] App Paths 收集时拒绝 helper/updater/installer/repair/crashpad 等名称标记
- [x] App Paths/Uninstall 收集时拒绝系统与包管理器目录下的 target（可测路径规则）
- [x] 通过启发式的条目仍可进入后续归并/兜底管线（本票不实现归并语义）
- [x] 纯函数/收集层单测覆盖接受与拒绝样例
- [x] `cargo check` / `cargo test` 通过
