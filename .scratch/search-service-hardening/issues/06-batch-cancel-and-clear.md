# 06: 候选验证批次取消 + 清空／隐藏取消任务

**Status:** resolved
**Blocked by:** 01（统一结果变更的失效与重提入口）
**Type:** performance / bugfix

**What to build:** 连续打字时，上一个查询的候选验证能在中途停下，新输入不用等旧任务把整批候选跑完；清空输入框或隐藏窗口时，后台也真的停下来，不用的时候不占 CPU。

- [x] 候选验证循环接受取消检查，按小批次（而非逐个候选）查询取消状态（`verify_all` 每 64 个候选查一次 `SearchRun::is_cancelled`，`VERIFY_CANCEL_BATCH`）
- [x] 既有阶段边界检查（召回后／验证后／排序前）保留
- [x] 清空 Query、隐藏窗口这两个场景，除清理界面状态外，还要通知 worker 停止当前任务（推进请求失效即可，不 kill 线程）——两个场景都经 `refresh_results` 的空 Query 分支调 `AppSearchWorker::cancel_current`，不在 hide／ClearQuery 各写一份
- [x] 取消语义不变：不返回部分结果（`verify_all` 返回 `None`）、不回调、不写缓存
- [x] 服务缝回归：用测试屏障固定「旧任务正在验证」这一刻，再提交新任务，断言旧任务被取消且不回调、不写缓存（`cancel_during_verify_skips_callback_and_cache_write`）

## Notes

- 槽位载荷改为 `Option<AppSearchJob>`：`None` 是「显式作废」——只推进代际，不产生任务。这样清空／隐藏不需要伪造一个任务，也不会在 `take()` 拿不到东西时让 worker 忙等。
- **屏障固定的是「刚进入候选验证」**（观察点在验证循环之前触发），所以该用例证明的是「取消后不回调、不写缓存」这条端到端契约。「按批次而不是只在进循环前查一次」由内核用例 `verify_all_stops_when_cancel_lands_mid_batch` 单独固定——该用例已做变异验证：把批次检查改成只在进循环前查一次，它会失败。
- UI 缝补了两个场景的回归（`clearing_query_cancels_in_flight_app_search` / `hiding_window_cancels_in_flight_app_search`）。除断言请求代际前进外，清空场景还断言「旧代际的结果即便送回来也不得落地」，避免只断言内部代际。

## Answer（提交 e8e6ff0）

- 取消从裸的 `&dyn Fn() -> bool` 收敛成 `SearchRun`：`is_cancelled()` 之外多一个 `entering_verify()`
  观察点，供测试用**屏障**（而不是 sleep 赌调度）固定「旧任务正在验证」这一刻。
- `verify_all` 每 `VERIFY_CANCEL_BATCH = 64` 个候选查一次取消；取消即返回 `None`——
  不返回部分结果、不回调、不写缓存，语义与改动前一致。
- 槽位载荷改为 `Option<AppSearchJob>`，`None` = 显式作废（只推进代际）；清空输入与隐藏窗口
  都走 `refresh_results` 的空 Query 分支调 `AppWorker::cancel_current`，不各写一份。

## Comments

- Spec 轴指出 UI 缝用例里的 `latest_seq()` 是内部代际，单靠它无法区分「真的停了算」。**部分采纳**：清空场景补上「旧结果不得落地」的外部可见断言；「隐藏」场景受 UI 缝能力所限（`hide` 之后旧回调本就被代际丢弃），只断言通知已发出，真实停算由服务缝用例负责。
- 提交前需按 `DEVELOPMENT.md` 做 Windows 运行验证（连续输入 + 回车、计算中清空、隐藏窗口）。本机无法运行 GUI，本票只完成编译与测试层验收，运行验证仍待补。
