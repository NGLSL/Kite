# ADR 0001：历史存储用 rusqlite（bundled）+ 手写 SQL

## 状态

已接受（Phase 3）

## 背景

Phase 3 需要持久化：启动次数、最近启动、Query→App 配对。PRD 要求「不要引入复杂重量级 ORM」。

## 决策

- 使用 **rusqlite + bundled**，库文件放应用数据目录
- 手写 SQL，表结构集中在 `storage/schema.sql` 逻辑里
- 连接由 `AppState` 用 `Mutex` 串行访问（启动器场景足够）

## 备选

- sqlx / diesel：更重，依赖与迁移成本高，当前阶段不需要
- 只存 JSON：查询与聚合（count / last_used_at）别扭

## 后果

- 编译时间变长（bundled sqlite）
- 历史表结构变更需手工 `PRAGMA user_version` 升级
