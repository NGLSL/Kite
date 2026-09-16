# 02: Bootstrap 只扫 A+B

**What to build:** 冷启动 Bootstrap 首屏只发布用户认为是应用的入口：Start Menu、Desktop、已配置的 Portable、curated system entries，并批量提图标。不再在 Bootstrap 收集 App Paths；Uninstall/Scoop/commands/protocol 仍留给 Full。用户首次安装看到的是干净可搜列表，而不是注册表 EXE。

**Blocked by:** 01 Portable 升为 Tier A 正式入口

**Status:** implemented

- [x] Bootstrap 扫描库存：不含 `app-paths` / `uninstall` / `scoop` / `commands`
- [x] Bootstrap 在已配置 portable_dirs 时索引 portable 正式入口
- [x] Bootstrap 仍发布 start-menu、desktop 与 curated system，且 retrieval 可搜
- [x] Bootstrap 批量提图标（含 portable 与 curated system，失败不拖垮）
- [x] Full 仍收集 App Paths/Uninstall 等来源（本票不改 Full 的 C 层策略）
- [x] 相关扫描库存测试通过
- [x] `cargo check` / `cargo test` 通过
- [ ] Windows 实机：Cold 有图标、无注册表杂讯
