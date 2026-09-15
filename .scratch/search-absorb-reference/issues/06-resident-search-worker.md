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
- 缓存仅在任务完整跑完且无个性化时写入。
