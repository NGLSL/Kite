# 05: 按键路径索引来源收敛

**Status:** resolved
**Blocked by:** None (can start immediately)
**Type:** performance

**What to build:** 连续打字时，每次按键的提交开销不再跟应用总数成正比——不再复制整份应用数组与系统入口数组，也不再为了取同一份状态加两次锁。

- [x] 搜索请求以「预建索引」为首选来源；只有在冷启动尚无预建索引时才使用后备数组（`IndexSource::Prebuilt` / `IndexSource::Snapshot`）
- [x] 有预建索引时，按键路径上不再无条件克隆完整的应用数组与系统入口数组（`ui/results.rs::request_app_search` 只在 `retrieval` 为 `None` 时克隆）
- [x] 索引状态一次读取即拿到自洽快照，不再出现两次加锁之间状态可能变化的情况（来源判定与数组快照在同一次加锁内完成）
- [x] 冷启动无预建索引的路径行为与改动前一致（同样的结果、同样的顺序）—— 服务缝 `prebuilt_and_snapshot_sources_agree` 固定
- [x] 主缝或服务缝测试覆盖「有预建索引」与「无预建索引」两条路径

## Notes

- 引入的小类型是 `IndexSource`，只有两个变体、没有缓存层，符合「不新增缓存层」的底线。
- 顺手修掉一处重复构建：后备快照原来在阶段 0 与阶段 1 各建一次索引，现在整次任务只建一次并共用。结果不变。
- **刻意不动**的一处既有边界：基础候选缓存无条件先按 `(index_generation, q_norm)` 查一次（两条来源都查）。理论上「先有预建索引代际 N 的缓存条目、之后同代际又以后备快照提交」会把旧 `doc_id` 套到新 build 的索引上；但索引重建会同时推进代际并清空缓存，该组合目前不可达，且本票的验收项明确要求冷启动路径行为与改动前一致，故保持原查找顺序，仅把行为差异写成这里的一条已知边界。

## Answer（提交 e8e6ff0）

- 新增 `IndexSource`（`Prebuilt(Arc<RetrievalIndex>)` / `Snapshot { apps, system_entries }`），
  搜索请求只带这一个来源；`request_app_search` 在**一次加锁**内取到自洽来源，
  有预建索引时只克隆 `Arc`，不再无条件复制整份应用数组与系统入口数组。
- 顺手去掉一处重复构建：后备快照原来在阶段 0 与阶段 1 各建一次索引，现在整次任务只建一次并共用。
- 冷启动（无预建索引）路径结果与顺序不变：服务缝 `prebuilt_and_snapshot_sources_agree`
  固定两条来源一致；缓存只在 `Prebuilt` 时写回。

## Comments

- Standards 轴建议把「`retrieval` 为 `Some` → 预建索引，否则克隆数组」这条映射提成构造函数（`IndexSource::from`）。**未采纳**：该映射只有一个构造点与一个消费点，消费点还要处理「是否可写缓存」与「快照只建一次索引」，再包一层只会变成中间人；`AGENTS.md` 要求避免无需求的抽象。
