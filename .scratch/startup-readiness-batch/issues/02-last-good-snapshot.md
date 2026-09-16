Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 02: last-good 索引快照

**What to build:** Full 成功完成后保存 last-good 快照；下次启动若快照版本/schema/代际兼容则立即载入并可搜索，跳过空窗。损坏或不兼容时安全丢弃，回退 Bootstrap+Full，不能启动失败。

**Blocked by:** 01 Bootstrap 首屏可搜索 + Full 原子替换

**Status:** ready-for-agent

- [ ] Full 成功后写入 last-good；Bootstrap/失败构建不覆盖 last-good
- [ ] 启动优先载入兼容的 last-good 并立即 publish 可搜索
- [ ] 版本/schema/代际校验失败则丢弃并走 Bootstrap，不 panic、不静默错绑 target
- [ ] last-good 只是索引快照缓存，不引入新的搜索数据库
- [ ] `cargo test` 覆盖：载入成功、损坏回退、Full 后更新
- [ ] Windows 实机：二次启动几乎立刻可搜；手动破坏快照后仍能自举
