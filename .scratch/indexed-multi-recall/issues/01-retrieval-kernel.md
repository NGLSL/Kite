# 01: 索引驱动多路召回内核

**What to build:** 非空 Query 在应用 + 系统入口快照上走统一多路召回（词元/前缀、n-gram、跳字位图、SymSpell、拼音），验证后统一排序；空 Query 的 Pin/Recency 不变。系统入口与应用共用评分，不在按键路径同步提取图标。

**Blocked by:** None (can start immediately)

**Status:** resolved

## Acceptance

- [x] RetrievalIndex 与 apps+system_entries 同代只读快照，扫描/UWP 合并后重建
- [x] 多通道召回取并集后逐候选验证，截断发生在统一评分之后（上限 200）
- [x] 系统入口（Kite 设置 / Windows 设置页 / 系统工具）与应用同层评分
- [x] 图标在快照准备阶段填充，匹配路径不提取图标
- [x] 既有搜索回归 + 新能力用例通过（中段片段、跳字、混合拼音、参考对照）

## Comments

- 2026-01: 已实现并提交 `514bebb`（分支 `feat/indexed-multi-recall`）。
- 词典暂用 BTreeMap、拼音用 `pinyin` 首读音、跳字自实现——与 spec 组件选型不一致，见票 02。
