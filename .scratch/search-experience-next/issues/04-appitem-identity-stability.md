Status: ready-for-agent
Type: task
Blocked by: 03 启动身份与展示优先级回归

## Goal

来源变化、快捷方式改名或安装路径迁移后，已有 Pin、用户 Alias 和 Query History 尽量继续指向同一可启动对象；升级不丢个人习惯，且历史不得误绑到不同 executable target。

## Scope

- AppItem 稳定 id 在来源变化时尽量保持；若 id 必须变化，迁移或兼容 Usage、Query History、Pin 与 Alias。
- 禁止把历史转给不同的可启动 target。
- 用户 Alias 优先于内置 Alias；歧义时不自动点名某一个应用。
- 不改变历史存储引擎（继续本地 SQLite）。

## Acceptance Criteria

- [ ] 同一程序从桌面/开始菜单/App Paths 多来源合并后，历史与 Pin 仍命中该首选 AppItem。
- [ ] 快捷方式改显示名但 target+args 不变时，Query History 与 Pin 仍有效。
- [ ] target 真正变化（不同 exe）时，不把旧历史加分错误套到新目标。
- [ ] 清空历史后空 Query 不再显示旧最近使用；暂停记录后不再新增。
- [ ] 现有设置、Pin、Alias 测试不回退。

## Validation

- `cargo test`（storage / history / identity 相关）
- Windows 实机：固定一个应用 → 改快捷方式来源或显示名 → 确认 Pin/Alias/History 仍指向可启动对象。

## Dependencies

- Blocked by: 03

## Out of Scope

- 云同步或身份图数据库。
- 召回特征与排序权重重调（票 07–09）。

## Comments
