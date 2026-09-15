# 08: demoted 表 user_version 迁移

**Status:** ready-for-agent
**Progress:** implemented

- [x] `SCHEMA_VERSION=2` + `migrate()` 前向迁移
- [x] 全新安装：表与版本正确
- [x] 老库无 demoted：升级并保留 usage 等旧数据
- [x] 表已存在、版本旧：迁移不丢降权记录
- [x] 不重构全部历史表
