# 07: Panel + NativeAction（Calculator 切片）

**What to build:** Provider 返回 panel 类型响应时，Kite 用声明式 Panel Schema 在启动器内原生渲染（text/value/key_value/notice/divider），搜索框保留。`=100*1.13` 或 `=1+2` 显示表达式与结果；存在 default action 时 Enter 执行 Native 动作（如 copy_text），**不**弹外部计算器窗口，也**不**为复制再打一次插件 RPC。

**Blocked by:** 05 — Plugin Host + JSON-RPC 生命周期  
（可与 06 并行开发）

**Status:** resolved

- [x] Panel Schema 校验：仅允许 V1 Block 类型；拒绝 HTML/自定义布局类字段
- [x] Iced 原生渲染上述 Block；Notice 支持 info/warning/error
- [x] default action + Enter → NativeAction（copy_text 等）由 Kite 直接执行
- [x] 插件不得重写 Esc 基础语义；Esc 退出 Provider Mode
- [x] 结果缝测试：Panel 映射、非法 block、Enter 复制不触发 plugin/execute
- [x] Calculator 协议级/官方插件：`=` Trigger + panel 响应（`official-plugins/calculator`，打包后 `resources/official-plugins/com.kite.calculator`）
- [ ] `cargo test` 全绿；有条件时真机冒烟计算复制（需本机 Python + GUI）

## Comments

- `panel_default_native_action_bypasses_stale_list_gate` 证明 Panel Native 复制不走 plugin/execute。
- 搜索结果区 `panel_area` 原生渲染 blocks。

## Notes

- 对应规格 Stage 5；测试主缝：结果缝。与 06 同属 Host 之上的展示/动作切片，无相互阻塞。
