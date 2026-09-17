# 02: 启动与动作路径统一走 ResultAction

**What to build:** 用户对列表结果按 Enter、Alt+数字、或鼠标点击时，Kite 解析该结果携带的 ResultAction 并执行对应行为：启动应用、打开文件/路径、打开 URL 等与现在一致；复制等动作有统一入口。旧列表在结果代际失效时仍不可启动（沿用既有「搜索代际 / 可启动资格」语义）。

**Blocked by:** 01 — Prefactor Expand — ResultSource + ResultAction

**Status:** ready-for-agent

- [ ] Enter / Alt+数字 / 行点击三条启动入口共用同一 ResultAction 判定，不再各自硬编码 AppItem 启动
- [ ] 应用结果 LaunchApp 行为与现状一致（只能使用索引中已验证 target，不拼 shell）
- [ ] 文件类结果可打开/定位，Web 结果可打开 URL
- [ ] 结果代际失效时，上述入口拒绝启动旧列表
- [ ] 真机冒烟：搜应用启动、文件模式打开、网页结果打开各一次
- [ ] `cargo test` 全绿；UI/键盘相关回归不回退

## Notes

- 对应规格 Stage 0 的 migrate 侧；是插件 List/Panel Action 的前置。
- 测试主缝：UI 启动有效性缝 + 既有 search 回归；不在本票引入插件进程。
