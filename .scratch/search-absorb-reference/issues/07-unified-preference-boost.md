# 07: 个性化加分单一计算函数

**What to build:** 抽出纯函数：偏好 + 使用 + Pin + 降权 + now → (boost, tags)。`history::apply_personalization` 与 `doc::apply_personalization_boosts` 共用；不合并两个结果类型。补回归：同 Query/索引/偏好快照下，缓存命中与不命中最终顺序一致（含保护层、证据同分、截断）。

**Blocked by:** None（下次碰 history 时）

**Status:** ready-for-agent

- [ ] 单一 boost/tags 计算入口，两路径共用
- [ ] 缓存命中 vs 重算顺序一致回归
- [ ] 不引入 trait/泛型框架
