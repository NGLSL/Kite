# 08: Plugin Manager UI

**What to build:** 设置页出现「插件」管理：列出已发现插件的名称、版本、启用状态与 Runtime 状态；用户可启用、禁用、重新加载、打开插件目录、查看日志、卸载。反复崩溃的插件被标出，用户可手动重新加载。管理操作反映到 Registry/Host，而不是只改本地开关文案。

**Blocked by:** 03 — Manifest Registry + Command 静态入口  
**Blocked by:** 05 — Plugin Host + JSON-RPC 生命周期

**Status:** resolved

- [x] 设置页列出插件及状态（含 Faulted/Incompatible/Disabled 等）
- [x] 启用/禁用：影响后续 Command/Provider 是否可激活；禁用后不再自动启动该插件
- [x] 重新加载：按当前 Manifest 重新注册并允许重新拉起进程
- [x] 打开目录、查看日志（stderr 日志可定位；日志 256KB 轮转为 .1）
- [x] 卸载入口：从管理列表移除（仅允许删除 plugins 根下目录）
- [x] Crash loop 提示可见，且支持手动恢复
- [x] UI/存储相关测试：启用/禁用、reload、uninstall 护栏
- [x] `cargo test` 全绿

## Comments

- 设置页 Section::Plugins + `plugins_card`。
- 管理操作写 Registry/Host，不是只改文案。
- 后续增强（非 V1 原文必做）：路径导入 + 安装官方示例 + 打开/重扫插件目录（`plugin/install.rs`）。

## Notes

- 对应规格 Stage 6；测试以设置状态机与 Registry/Host 交互缝为主。
- V1 不做 Plugin Store；本地文件夹导入与官方样例 seed 已补齐「怎么装进 plugins/」。
