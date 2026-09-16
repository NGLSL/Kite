Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 03: 初始构建期间 watcher 延迟 pending

**What to build:** 入口 watcher 与初始 Bootstrap/Full 同时存在时，首屏发布完成前到达的脏事件只记 pending，不在初始构建进行中叠跑第二轮 Full；首屏/初始阶段发布后再消费 pending。避免安装器或目录噪音在冷启动期间双跑完整扫描。

**Blocked by:** 01 Bootstrap 首屏可搜索 + Full 原子替换

**Status:** ready-for-agent

- [ ] 明确「初始索引已发布」边界（Bootstrap 发布与 Full 完成的约定写清）
- [ ] 初始构建期间 FS/注册表脏事件合并为 pending，不启动并行 Full
- [ ] 初始发布后若有 pending，再触发一轮必要重建（仍 single-flight）
- [ ] 运行中安装/卸载仍能在约 5s 量级看到更新（既有监听语义不回退）
- [ ] 日志可区分：初始构建中忽略的脏事件 vs 发布后的 rebuild
- [ ] `cargo test` 覆盖时序；实机冷启动 + 同时批量写目录不持续重扫
