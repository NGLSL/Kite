Status: resolved
Type: task
Blocked by: 06 搜索评估基线

## Goal

先按基础相关性划分候选竞争层级，再在同层或相近质量候选间应用 Query History、使用次数与近期使用；History 不得把无关弱匹配推到明确匹配之前；清空/暂停历史后排序立即反映用户控制。

## Scope

- 保留 MatchScore 为主信号、明确匹配保护、Pin 与历史取较大者不叠加越界。
- 历史只在相近质量层内调整顺序，不为完全无关候选补分。
- 同一程序多入口展示优先级可单独处理，不无条件降低所有精确匹配。
- 用户 Alias、Pin 继续生效。
- 以固定测试集的 Top1 与 MRR 改善为准重调权重，不因单产品截图调分。

## Acceptance Criteria

- [x] 多次用 `ter` 打开 XTerminal 后，它在相关性接近的终端候选前移，但无关弱匹配不因此进入首屏前列。
- [x] 明确 exact / word-prefix 匹配不会被低相关 substring/fuzzy + 历史压过。
- [x] 清空历史或暂停记录后，后续排序立即不再受益于旧加分。
- [x] Pin 保护边界仍满足：Prefix+个性化 < Name Exact 等不变量。
- [x] 相对票 06（及 07/08 若已合入）基线：Top1 与 MRR 改善或持平，重复入口数不恶化。

## Validation

- `cargo test`（history / ranker / search_cases，含隔离历史库）
- 对比基线指标。

## Dependencies

- Blocked by: 06
- 依赖票 07/08 的样本期望最终形态以便公平对比；若 07/08 未完成，先在当前召回上分层，再在合入后复测。

## Out of Scope

- 新个性化模型或云端学习。
- 全量 LRU 排序。

## Comments

2026-09-14 实现记录：

- `quality_tier(base_score)`：user-alias / name-exact / compact / token / prefix / pinyin / word-prefix / substring / fuzzy 分层。
- `apply_boosts` 先按基础分算层，层内用 History/Pin 调序，再按 (tier, score) 重排；History 不能跨层抬升弱匹配。
- 单测：`history_cannot_lift_substring_above_word_prefix`、`history_reorders_within_same_tier`。
- 清空/暂停历史行为沿用 storage 既有测试。
- `cargo test` 179 passed。
