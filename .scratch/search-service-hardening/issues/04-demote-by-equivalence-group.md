# 04: 降权按等价展示组统一解释

**Status:** resolved
**Blocked by:** None (can start immediately)
**Type:** bugfix

**What to build:** 对一个有多个等价扫描入口的应用（同一安装、同一启动动作、已确认可合并）执行「降低此结果优先级」，那一行确实后移；执行「恢复优先级」后回到原顺序；无论降权前后，启动的还是同一个目标。

- [x] 已确认等价的展示组内成员一致受该组降权影响，且不做重复累加（`group_demote_is_not_stacked_and_is_restorable`）
- [x] 展示代表的选择与降权解释共用同一份等价组信息，不新增第二套等价判定（`launch_rep` / `launch_members`，抽出 `group_rep_of` / `group_members_of` 供两处共用）
- [x] 同组存在等基础分成员时，降权后展示行的分数确实下降——不再被组内另一入口把分数顶回原值（`demote_applies_across_equivalent_launch_members`）
- [x] 明确匹配保护层不被降权挤出（`quality_tier <= PROTECTED_TIER_MAX` 时不扣分，「我就是要搜这个名字」仍然有效）
- [x] 降权按名称扩大范围不发生；不删除索引项、不改启动目标（测试断言降权前后 `target` 不变）
- [x] 主排序缝回归：同组等基础分场景下，降权后该行后移，恢复后回到原序（`demote_group_falls_behind_equal_peer_from_another_install`）

## Notes

- 当前行为是 no-op：归并时展示代表恒取静态主入口，但分数来自组内排名最好的成员，于是同分成员会把降权后的分数顶回原值，标签也不显示。用户点了降权却看不到任何变化。
- **不撤销归并、不恢复同一应用的两行展示（如两行微信）**——本票只修「降权被抵消」。
- 不按软件名称或 exe 名称扩大降权范围。

## Answer（提交 279e4b8）

- 降权改按**展示组**统一解释：组内成员一致受影响，且不重复累加；明确匹配保护保留，
  归并结果不变（不多出一行微信）。
- 修复了原先「展示代表取静态主入口、分数取组内最好成员」导致的分数被顶回原值问题。
- 回归测试：`demote_applies_across_equivalent_launch_members`、
  `demote_group_falls_behind_equal_peer_from_another_install`、
  `group_demote_is_not_stacked_and_is_restorable`（`src/search/tests.rs`）。
- 同提交按 spec 的文档同步要求，修订了
  `.scratch/search-absorb-reference/issues/03-search-service-cancel-generation-cache.md`
  与 `06-resident-search-worker.md` 里已过时的缓存语义描述。

未验证项：真机需实测「对有多个等价入口的应用降权后该行确实变化」。
