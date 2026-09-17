# 02: Index Health 状态与日志

**What to build:** Full 索引成功/失败、last-good 年龄、是否来自 aged Warm、连续失败次数可被记录与读取；日志能看出「健康 / 快照过旧 / 完整扫描失败」。Warm 启动策略不变——aged 仍恢复，不硬拒。

**Blocked by:** 01 统一 JSON 原子写

**Status:** resolved

- [x] `src/app/index_health.rs`：`IndexHealth`（success/failure 时间、原因、streak、warm age、aged）
- [x] Full 成功/失败挂钩 `backend::queue_full_snapshot`（以 last-good 写入成败为准；scanner 仍不返回 Result）
- [x] Warm load 在 `snapshot::load_from` 记 age/aged；aged 仍返回可用索引
- [x] 损坏/缺失状态文件 → 默认；经 `atomic_file` 写盘
- [x] 单测 6 项通过；`cargo test -- --test-threads=1` 435 passed / 9 ignored

## Comments

- 运行时目录与 last-good 同级：`index-health.json`。
- `summary()` 已备好给票 03 设置页展示。
- 扫描过程本身不返回 `Result`，本票 Full 失败以 snapshot save 失败为观测点。
