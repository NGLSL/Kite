# 03: 匹配证据细排

**What to build:** 同一种匹配也能分好坏。候选验证保留匹配证据（命中字段、匹配方式、起点/连续段/间隔/覆盖比例、纠错代价），最终排序依据这些证据，而不是每个新规则再加一个固定分常量。同等可信度下，完整词、词边界、连续且紧凑的匹配更靠前；偶然词中片段与高代价纠错靠后。拼音位置映射回原名称再比较。多路通道命中同一段内容时证据可保留供解释，分数不重复叠加成虚高。用户能感到「同样是片段，排得有道理」。

**Blocked by:** 01 统一最终排序管线

**Status:** done

- [x] 候选排序使用完整匹配证据；展示可用简化 matched_by，但排序不只依赖单一固定分标签
- [x] 成对样本断言相对顺序：连续 vs 跳开、词边界 vs 词中、精确/低代价 vs 高代价纠错
- [x] 拼音匹配位置映射回原名称字段后再参与比较，不与中文名位置直接混比
- [x] 同段多路命中不重复贡献分数；用户 Alias 与明确名称仍受保护
- [x] 无产品名专属打分分支；相对票 01 后基线 Top1/MRR 改善或持平，无样本显著回退
- [x] `cargo test` 全绿，search_cases 相关样本通过

## Comments

2026-03-12 实现记录：

- `MatchEvidence { field, kind, start, span, gaps, edit_cost }`；`ScoredHit` 携带证据。
- `take_best` 同分用 `outranks`：更早起点 → 更少跳空 → 更低编辑代价 → 更长 span → 字段 → 匹配方式。
- 拼音首字母中段 `map_initial_index_to_display` 映射回 display 字符下标。
- 最终候选排序：score → evidence → doc_id。
- 单测：`evidence_prefers_earlier_and_tighter_match`、skip 返回 gaps。
- 多路仍 `take_best` 取最优，不叠加同段分。
- `cargo test` 284 passed；eval required 全过。
