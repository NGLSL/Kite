# 06: List + PluginAction（Window Switcher 切片）

**What to build:** Provider 返回 list 类型响应时，条目以 Kite 原生列表展示（标题/副标题/图标等）。用户按 Enter 对选中项执行 plugin_action（如 activate_window），Kite 通过 plugin/execute 调用插件完成动作。场景：`win kite` 在 Kite 列表中出现窗口候选并可激活，不弹独立「切换器 UI」抢框架。

**Blocked by:** 05 — Plugin Host + JSON-RPC 生命周期

**Status:** resolved

- [x] List 响应映射为通用 Result（source=Plugin，携带 plugin_id/provider_id）
- [x] priority 仅影响当前 Provider 内部顺序，不影响 Core Ranking
- [x] Enter → ResultAction::Plugin → plugin/execute(action_id, payload)
- [x] Native 与 Plugin 动作分离：List 场景走 PluginAction；不把 Window 当成特殊 QueryResponse
- [x] 结果代际失效时不可对过期插件行执行动作
- [x] 结果缝测试：List 映射与 execute 参数；官方 Window Switcher 在 `official-plugins/window-switcher`
- [ ] `cargo test` 全绿；有条件时真机冒烟 `win` 流程（需用户本机窗口环境）

## Comments

- 协议层 `ListItemAction::PluginAction`；宿主回填 plugin_id。
- `window_list_provider_maps_plugin_actions` 覆盖结果缝。

## Notes

- 对应规格 Stage 4；测试主缝：结果缝 + 运行时缝已有能力。
