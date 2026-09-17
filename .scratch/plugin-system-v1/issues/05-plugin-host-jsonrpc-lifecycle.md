# 05: Plugin Host + JSON-RPC 生命周期

**What to build:** Provider Mode 需要数据时，Plugin Host 懒启动对应插件子进程，完成 initialize 与 query，并把声明式响应交回上层；随后请求复用同一进程，空闲超时退出。插件慢、挂起或崩溃时 Kite UI 不冻结、不退出；过期 generation 的响应被丢弃。进程通信使用 JSON-RPC 2.0 over stdio + Content-Length 帧；日志走 stderr。

**Blocked by:** 04 — Activation Router + Provider Mode

**Status:** ready-for-agent

- [ ] Runtime 状态机：Dormant → Starting → Ready；Faulted / Incompatible / Disabled 可观测
- [ ] 懒启动：未触发前 0 进程；首次触发 spawn；同一插件多 Provider 共用一个进程
- [ ] 禁止每次按键 spawn；后续 Query 走 RPC 复用
- [ ] Initialize/Query 硬超时（对齐规格：init 目标 <500ms/硬 2000ms；query 硬 800ms）；超时只丢弃本次结果
- [ ] Hang：插件 sleep 时调用方不阻塞 UI 线程；用户仍可搜索并启动应用
- [ ] Crash：进程退出 → Faulted，Kite 继续运行；不自动后台重启；crash loop（短时间多次）抑制自动拉起
- [ ] Idle Shutdown：达到 clamp 后的 timeout 进程退出
- [ ] generation 过期响应丢弃；cancellation 可选，不作为正确性前提
- [ ] Content-Length 帧与 plugin/initialize、plugin/query（及 execute 签名）消息形状可测
- [ ] 运行时缝（可注入 ProcessBackend / mock stdio）覆盖上述生命周期；不依赖真实第三方 exe 作为唯一手段
- [ ] `cargo test` 全绿

## Notes

- 对应规格 Stage 3；测试主缝：运行时缝。Host API 本票只定契约与最小实现，能力面保持极少。
- RPC 不得跑在 Iced UI 线程。
