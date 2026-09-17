# 01: 统一 JSON 原子写

**What to build:** 索引快照与全部 JSON 侧缓存（LNK `ScanCache`、`UWP` 缓存、exe `MetaCache`）在 Windows 上走同一套原子替换写盘；进程在保存中途被杀不会留下「先删后改名」的空窗；写失败写日志且不阻断扫描。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] 抽出公共原子写入口 `app::atomic_file::write`；snapshot 的 `ReplaceFileW` / `MoveFileExW` 实现被复用
- [x] `ScanCache` / `UwpCache` / `MetaCache` 的 save 接到该入口，去掉各自 remove+rename 与静默 `let _ =`
- [x] 覆盖已有目标文件时保存成功，再 load 内容正确（四类缓存/snapshot + `atomic_file` 自测）
- [x] 写失败可观测：日志含目标路径（`lnk/uwp/meta cache write failed path=…`）；调用方不 panic
- [x] `cargo test`：438 中 427 过 / 9 ignored；2 个 portable/bootstrap 预算类用例并行 flaky，串行复跑通过，与本改动无关

## Comments

- 实现：新增 `src/app/atomic_file.rs`；`snapshot::save_to`、`cache.rs`（LNK/UWP）、`metadata.rs`（Meta）共用。
- MetaCache 新增 `save_overwrites_existing_meta_cache`。
- 并行全量下 `configured_portable_directory_is_indexed` / `bootstrap_pass_skips_scoop_and_command_sources` 可能因本机 Start Menu 过大、timed budget 在到 portable 前耗尽而失败；`--test-threads=1` 下稳定通过。
