Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 04: 启动阶段性能基线指标

**What to build:** 性能脚本在既有唤起延迟/内存之外，新增并汇总 `time_to_window_ready`、`time_to_first_searchable`、`time_to_full_index`、`post_index_settle_cpu`，让「首次可搜索」和「完整索引」不再被 P50 唤起延迟掩盖。

**Blocked by:** 01 Bootstrap 首屏可搜索 + Full 原子替换

**Status:** ready-for-agent

- [ ] 脚本能分别记录窗口就绪、首次可搜索、完整索引完成的耗时
- [ ] Full 后有 settle CPU 观察窗口，确认索引稳定后无异常空转
- [ ] 保留既有 open_ms / 内存等指标，便于防回归对比
- [ ] 输出可读 summary，便于后续基线对比
- [ ] 在当前实现上跑通一次并记录结果（不必为本轮强行改数值）
