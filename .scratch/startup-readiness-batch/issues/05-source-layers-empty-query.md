Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 05: 来源分层 + 空 Query 隐藏命令裸行

**What to build:** 索引中的 `AppItem` 具备来源层级：正式入口（Start Menu/Desktop/UWP）、补充发现（App Paths/Uninstall）、命令 Alias（Scoop/WindowsApps/WinGet/Chocolatey）。空 Query 默认不显示命令裸行，除非 Pin 或最近使用。不按 System32 等路径前缀硬过滤。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] 来源在扫描/模型侧可区分三层，且不删除命令来源索引
- [ ] 空 Query 列表默认无命令裸行
- [ ] Pin 或 Recency 的命令入口在空 Query 仍可见
- [ ] 正式入口、系统内置入口不受此隐藏影响
- [ ] 不引入产品名特例，不硬杀路径前缀
- [ ] `cargo test` 覆盖空 Query 可见性策略（结果缝）
- [ ] 实机：空列表更像应用启动器，而不是 PATH 浏览器
