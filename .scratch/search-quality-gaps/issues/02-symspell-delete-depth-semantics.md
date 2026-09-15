# 02: 修正纠错通道删除深度语义

**What to build:** 用户输入接近真实词元的拼写（如词典有 `chrome`、输入 `crome`）时，纠错召回通道不得把「查询侧删除深度为 0」误当成「不是纠错」而丢掉候选。删除索引返回的次数是查询侧生成删除变体的深度，不是与词典原词的真实编辑距离；候选是否符合纠错距离，由验证阶段用真实代价决定。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] 纠错通道不再因查询侧删除次数为 0 而跳过候选
- [x] 语义区分：查询侧删除深度 ≠ 真实编辑距离；若保留「纯精确命中不走纠错」短路，必须基于输入与词典原词相同，而不是深度 0
- [x] 端到端样本：索引含名称可映射到 `chrome` 的应用时，输入 `crome` 能出现在结果中（允许非首位，不得因通道自删而消失）
- [x] 若需要，辅助单测固定 `DeleteIndex::expand`：直接命中删除变体时返回深度 0，且词典原词在返回集中
- [x] 无产品名专属打分分支
- [x] `cargo test` 全绿

## Comments

2026-03-12 实现：

- `channels::collect` SymSpell 两处 `if dist == 0 { continue }` 删除；注释写明 dist 为查询侧删除深度
- 通道单测 `direct_delete_variant_is_not_dropped_by_symspell_channel`（stats.symspell > 0）
- e2e `deletion_typo_direct_variant_still_recalls_original`（fuzzy 也会兜底，通道语义以单测为准）
