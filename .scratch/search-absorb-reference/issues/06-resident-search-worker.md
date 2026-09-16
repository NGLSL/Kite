# 06: 常驻搜索线程 + 协作取消

**Status:** ready-for-agent
**Progress:** implemented

- [x] 全局仅一个搜索 worker 线程（`AppSearchWorker::spawn`）
- [x] 待处理请求只保留最新一份（`LatestSlot` 覆盖）
- [x] 旧任务在阶段边界协作退出（召回后/验证后/排序前；`search_personalized_cancellable`）
- [x] UI 仍保留代际/查询文本双检；取消产物不进缓存、不回调
- [x] 服务缝测试：latest-wins、取消返回 None、slot 覆盖
- [x] `cargo test` 全绿

## Notes

- 取消信号 = slot 代际前进；不强行 kill 线程。
- 缓存写入的是**个性化前**的轻量候选（未截断），个性化每次按最新偏好重放；
  与是否有历史/Pin/降权无关，不再有「仅无个性化时才写入」的限制。
- 写入还需通过缓存失效代际（`BaseHitCache::insert_if_epoch`）：别名一类基础候选
  依赖变化后，在途任务完成也不得把旧结果写回。

> 本节于 `search-service-hardening` 工单 03 修订：原文「缓存仅在任务完整跑完且无个性化时写入」与实现矛盾。
