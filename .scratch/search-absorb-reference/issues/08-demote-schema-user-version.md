# 08: demoted 表 user_version 迁移

**What to build:** 按 ADR 0001 将 demoted（及核对 pinned）纳入 `PRAGMA user_version` 迁移。覆盖：全新安装；老库无 demoted；表已存在但版本旧。不删表、不丢降权记录；不重构全部历史表。

**Blocked by:** None（下次碰 storage 时）

**Status:** ready-for-agent

- [ ] 全新安装 schema+版本正确
- [ ] 老库升级保留数据
- [ ] 已存在表可完成迁移
