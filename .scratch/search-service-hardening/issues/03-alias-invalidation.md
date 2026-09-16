# 03: 别名变化使基础候选缓存与在途请求失效

**Status:** resolved
**Blocked by:** 01（统一结果变更的失效与重提入口）
**Type:** bugfix

**What to build:** 用户把别名 `qa` 从应用 A 改成指向应用 B 之后，再搜 `qa` 就按 B 排，不需要重启应用或重新扫描。同时，改别名之前就已发出的那次搜索，跑完了也不会把 A 的结果重新塞回来。

- [x] 缓存的有效性条件包含影响基础候选的偏好输入，至少覆盖别名目标集合 —— 实现取「失效条件」路线：缓存失效代际 `BaseHitCache::epoch`
- [x] 别名增删改时，基础候选缓存被清空，且请求失效代际同步推进（`refresh_after_alias_change`）
- [x] 旧任务完成后不得把用旧别名算出的结果写回当前缓存（`insert_if_epoch`，代际判断与 `invalidate` 共用同一把锁）
- [x] 服务缝回归：`invalidate_rejects_writeback_from_in_flight_search`；另有主缝 `alias_target_change_reorders_results`、UI 缝 `alias_change_invalidates_base_cache_and_resubmits`
- [x] 不存在「只清数据、不推进代际」的失效路径（别名路径统一走 `invalidate`；`clear` 只留给索引重建，其缓存键已含索引代际）
- [x] 同步修订仍描述旧缓存语义的说明（本轮补上：旧工单 03 的验收项、旧工单 06 的 Notes、`BaseHitCache` 文档、`State` 里 `base_hit_cache` 字段注释）

## Notes

- 用户别名属于明确匹配保护层，命中会稳定排在前面，所以旧别名结果会以「很显眼」的方式错下去——用户会直接判定「设置没生效」。
- 复用 01 的失效入口，不新建第二套缓存失效机制。
- 缓存语义固定为「个性化前候选」，个性化每次按最新偏好重放；本票只负责让别名变化能进来。

## Answer（提交 279e4b8）

- 新增 `refresh_after_alias_change(state)`（`src/ui/actions.rs`）作为别名增删改的唯一出口：
  复用 01 的失效入口清空基础候选缓存，并重提交请求，**在途的旧请求因此一并作废**
  （不会出现「旧任务跑完把旧别名结果写回当前缓存」）。
- 别名目标变化后结果按新别名重排。回归测试：
  `alias_target_change_reorders_results`、`alias_change_invalidates_base_cache_and_resubmits`
  （`src/search/tests.rs` / `src/ui/results.rs`）。

未验证项：真机需实测「改别名后立刻重搜」。
