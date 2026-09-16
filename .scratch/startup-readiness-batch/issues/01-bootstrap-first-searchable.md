Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 01: Bootstrap 首屏可搜索 + Full 原子替换

**What to build:** 冷启动无 last-good 时，先用 Bootstrap Pass 扫高价值正式入口（Start Menu / Desktop / App Paths / 内置系统入口），附着 search fields 并发布可搜索 `AppIndex`；随后后台 Full 补齐 metadata / UWP / icon / 全来源，完成后只原子替换一次。用户一打开 Kite 就能搜到绝大多数软件，Full 再慢也不阻塞首屏。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] 新增 Bootstrap 扫描阶段：仅正式高价值来源，不做图标提取、metadata、UWP、uninstall、Scoop/commands、protocol
- [ ] Bootstrap 不得原样复用生产上的旧 Fast 来源顺序；Fast 不得再作为生产首屏路径
- [ ] Bootstrap 完成后立即 publish 可搜索快照并通知 UI 刷新
- [ ] Full 在后台继续，完成后原子替换；Full 未完成前不发布不完整快照
- [ ] 同一时刻仍最多一次在跑的索引构建；Bootstrap 与 Full 共享 single-flight
- [ ] 单条坏快捷方式/不可读目录不导致整份索引失败
- [ ] `cargo check` / `cargo test` 通过；相关扫描与发布时间线回归覆盖
- [ ] Windows 实机：冷启动先可搜到开始菜单/桌面常见应用，稍后深层/UWP 等补齐且列表不反复空跳
