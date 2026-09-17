# 08: Plugin Manager UI

**What to build:** 设置页出现「插件」管理：列出已发现插件的名称、版本、启用状态与 Runtime 状态；用户可启用、禁用、重新加载、打开插件目录、查看日志、卸载。反复崩溃的插件被标出，用户可手动重新加载。管理操作反映到 Registry/Host，而不是只改本地开关文案。

**Blocked by:** 03 — Manifest Registry + Command 静态入口  
**Blocked by:** 05 — Plugin Host + JSON-RPC 生命周期

**Status:** ready-for-agent

- [ ] 设置页列出插件及状态（含 Faulted/Incompatible/Disabled 等）
- [ ] 启用/禁用：影响后续 Command/Provider 是否可激活；禁用后不再自动启动该插件
- [ ] 重新加载：按当前 Manifest 重新注册并允许重新拉起进程
- [ ] 打开目录、查看日志（stderr 日志可定位；日志有大小限制/轮转约定）
- [ ] 卸载入口：从管理列表移除（文件删除策略按插件目录约定执行，不破坏其他插件）
- [ ] Crash loop 提示可见，且支持手动恢复
- [ ] UI/存储相关测试或可脚本化验证：状态展示与启用/禁用切换
- [ ] `cargo test` 全绿

## Notes

- 对应规格 Stage 6；测试以设置状态机与 Registry/Host 交互缝为主。
- V1 不做 Plugin Store。
