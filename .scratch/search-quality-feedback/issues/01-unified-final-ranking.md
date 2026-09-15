# 01: 统一最终排序管线

**What to build:** 非空 Query 的应用与系统入口搜索走同一条最终排序管线：多路召回 → 候选验证 → 读取 Query 偏好/历史/Pin → 统一计算质量与偏好 → 一次排序并截断 Top N → 物化展示对象。用户在接近质量的候选中选择后，偏好有机会作用于更宽候选集；完整名称精确匹配与用户 Alias 等明确匹配保护贯穿最终列表，不被层排序后的纯分数重排抹掉。空 Query 默认列表、网页搜索槽位与文件结果边界保持可用。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] 非空 Query 主结果（应用 + 系统入口）只经一条最终排序截断；UI 不再「检索截断后再加分」或「层排序后再纯分数重排」
- [x] 质量层级是显式排序字段（或等价第一排序键），不靠加分后数值区间反推
- [x] 个性化状态在截断前参与排序；注入接近质量候选后，被旧流程提前截断的目标仍可进入 Top N 并受偏好影响
- [x] 明确匹配保护不变量仍成立：Name Exact / 用户 Alias 不被 History/Pin 从下方抬升的弱匹配压过
- [x] Pin 与历史取较大者、不叠加越过保护界限；空 Query 行为不回退
- [x] 统一入口成为主测试缝；`cargo test` 全绿，既有 search/history 样本不恶化

## Comments

2026-03-12 实现记录：

- `SearchResult` 增加显式 `quality_tier`；`SearchResult::scored` 在物化时写入。
- `history::Personalization` + `apply_personalization`：在完整候选上按 (tier, FinalScore) 重排；`apply_boosts` 委托同一实现。
- `RetrievalIndex::search_personalized` / `search::search_with_personalization` / `search_system_personalized`：个性化在截断前生效；有个性化时物化全部已验证候选。
- `rank_and_truncate` 以质量层为第一排序键。
- `prefer_friendly_install_entries` 折扣后重算 `quality_tier`，避免 app-paths 精确分残留高层。
- storage 增加 `usage_all` / `query_pairs_for`，供截断前个性化快照。
- UI `refresh_results` 走统一入口，去掉检索后的 `apply_boosts` + 纯分数 `rerank`。
- 新增测试：显式层、截断前偏好抬升、Name Exact 保护、Pin 不叠压 exact。
- `cargo test`：279 passed, 0 failed。
