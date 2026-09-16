Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 07: 搜索视图收口（热键 / 路径 / 滚动 / 右键）

**What to build:** 一次收掉结果视图四类问题：footer 托盘显示当前生效热键（不再写死 Alt + Space）；hover/selected 显示 target 路径第二行且行高不变；scrollbar 右侧留 gutter 不挡内容；根视图始终 Stack，右键开关菜单不再改变列表 widget 位置、不滚回顶部。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] footer 绑定当前 hotkey label；设置修改后与真实热键一致
- [ ] 托盘提示热键与当前热键一致
- [ ] hover 或 selected 显示路径副标题：弱化、省略、行高保持
- [ ] 列表为 scrollbar 预留 gutter，右侧操作区不被遮挡
- [ ] 根 Stack 常驻：无菜单 overlay 为空，有菜单只替换 overlay
- [ ] 右键打开/关闭菜单后列表保持原滚动位置
- [ ] `cargo check` / `cargo test`；实机核对上述可见行为
