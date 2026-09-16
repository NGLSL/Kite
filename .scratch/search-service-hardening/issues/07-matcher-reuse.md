# 07: matcher 与工作缓冲跨查询复用

**Status:** resolved
**Blocked by:** None (can start immediately)
**Type:** performance

**What to build:** 连续输入多个查询时，匹配器与工作缓冲不再每个查询重新建一次，省掉重复的初始化开销。

- [x] worker 持有匹配器与工作缓冲，查询只更新 pattern 与解析结果（worker 线程持有 `MatcherScratch`，`QueryContext::build` 只重建 ib-pinyin matcher 与 nucleo pattern）
- [x] 复用跨查询生效，不再只在同一次查询的候选之间复用
- [x] 改动前后同一批查询的结果与顺序一致（主缝回归不回归）—— 服务缝 `reused_scratch_matches_fresh_scratch_across_queries` 逐个查询比对「复用工作区」与「每次新建」的结果与分值
- [x] 不新增抽象层——这是把既有缓冲上移到 worker 持有，不是新设计

## Notes

- `MatcherScratch` 只是给既有的两个缓冲（`NucleoMatcher` + `Vec<char>`）起个名字并随线程持有，没有间接层、没有池化。之所以要有个名字：否则服务层就得自己 `NucleoMatcher::new(NucleoConfig::DEFAULT)`，把 matcher 的构造知识漏到服务层。
- 一次性入口（`search_personalized_cancellable`、`search_personalized_ranked`、`reference_search`、测试）仍各建一份用完即弃，只有常驻 worker 跨查询复用。
- 顺手删掉一处浪费：`reference_search` 原来为了给 `QueryContext::build` 凑一个（当参数不用的）索引，每次调用都 `RetrievalIndex::build(&[], &[])`。

## Answer（提交 e8e6ff0）

- `MatcherScratch { nucleo, hay_buf }` 由常驻 worker 持有，随任务传给 `verify_all`；
  `QueryContext::build(q, scratch)` 只更新 pattern 与解析结果，不再每个查询重建 matcher 与字符缓冲。
- 复用范围是**跨查询**，不再只在同一次查询的候选之间复用。
- 结果不变：服务缝 `reused_scratch_matches_fresh_scratch_across_queries` 逐个查询比对
  「复用工作区」与「每次新建」的结果与分值。
- 顺手删掉 `reference_search` 里为凑参数而做的 `RetrievalIndex::build(&[], &[])`。

## Comments

- Standards 轴把 `SearchRun::with_verify_hook` / `entering_verify` / `spawn_with_verify_hook` 标为「只为测试存在的推测性通用」。**未采纳删除**：06 的验收项明确要求「用测试屏障固定旧任务正在验证这一刻」，不在 worker 里留一个观察点就只能靠 sleep 赌调度，与验收项直接冲突。生产路径走的是 `None` 分支，代价是一次 `Option` 判断。
