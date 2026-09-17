# 03: 设置页索引健康只读展示

**What to build:** 设置页只读展示一行索引健康信息，文案来自 Index Health（正常 / 快照过旧 N 小时 / 完整扫描失败 N 次等），不阻塞 UI、不提供复杂操作。

**Blocked by:** 02 Index Health 状态与日志

**Status:** resolved

- [x] 设置页「应用索引」卡片新增只读「索引健康」行（与重新扫描/当前索引同卡）
- [x] 文案来自 `IndexHealth::summary`：正常 / 快照过旧 / 完整扫描失败 N 次
- [x] 未知状态显示「索引状态未知」，不写成已损坏；右侧「正常 / 需关注」
- [x] `cargo check` 通过；`index_health` 单测覆盖三类文案

## Comments

- 实现：`settings_view::index_card` 读 `index_health::load()` + `summary(now)`，只读无操作。
- 实机打开设置 → 应用索引 可见；本会话未做 GUI 截图验收。
