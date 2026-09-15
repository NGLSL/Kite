# 01: 贯穿匹配证据的最终排序

**What to build:** 用户在最终列表里看到的同分顺序，必须由验证阶段算出的匹配证据决定：起点更早、间隔更小、连续且紧凑的命中稳定靠前。有无查询偏好时同分规则一致；稳定 id 或名称长度不得在证据之前覆盖胜负。个性化加分仍作用在截断前的完整候选上；物化展示对象（克隆 AppItem）只发生在最终排序并截断之后。上一轮已修好的「偏好在截断前生效」「Name Exact / Alias 明确匹配保护」不得回退。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] 验证后的轻量候选（稳定 id、基础 MatchScore、质量层、匹配证据）一路保留到最终统一比较，不提前丢弃证据
- [x] 最终比较器统一为：质量层 → 最终分（含偏好）→ 匹配证据细排 → 稳定 id；无个性化路径不再让名称长度/名称优先于证据
- [x] 有无 `Personalization` 两条路径使用同一证据比较键
- [x] 有个性化时不在截断前对全部已验证候选做多余的 AppItem 物化/重复克隆
- [x] 成对样本：同分且证据更优者第一，即使其稳定 id 更大；无个性化与有个性化各测一次
- [x] 既有样本不回退：截断前 Query 偏好仍能抬升相近候选；Name Exact 不被强历史压过
- [x] `cargo test` 全绿

## Comments

2026-03-12 实现：

- `RankedHit` 保留 `doc_id / score / quality_tier / matched_by / evidence / name_len / name_lower / stable_id`
- `search_personalized`：verify → RankedHit → friendly discount → 可选 personalization → `cmp_ranked_hit` → truncate → 物化
- 保护层分区保留，层内同一比较器
- 测试：`same_score_prefers_earlier_match_*`；全量 `cargo test --lib` 300 passed
