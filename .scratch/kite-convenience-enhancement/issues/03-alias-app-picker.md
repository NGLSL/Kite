Status: resolved
Type: task
Blocked by: 01

## Goal

把用户 Alias 的目标从自由文本输入改成可选择、可校验的 AppItem。

## Scope

- 输入目标名称时返回索引候选 Top N。
- 选择候选后保存规范显示名和稳定目标标识。
- 保存前由 Rust 校验目标存在。
- 兼容已有仅保存 target_name 的 Alias 数据。
- 目标不存在时显示明确错误，不保存无效 Alias。

## Acceptance Criteria

- 输入目标名称可以看到候选应用。
- 选择候选后 Alias 能稳定命中对应 AppItem。
- 重名应用不会仅靠显示名称静默绑定到错误目标。
- 旧 Alias 可以继续读取或被安全迁移。
- Alias 新增、修改、删除后搜索缓存立即更新。
- IPC 只返回候选 Top N，不传完整应用索引到 React。

## Validation

- `cd src-tauri; cargo test`
- `npm run build`
- Windows 实机检查普通应用、UWP 和同名候选。
- 检查错误目标、旧 Alias 和删除 Alias 的行为。

## Dependencies

- 依赖 issue 01 的 Alias 缓存失效行为。

## Out of Scope

- 自动学习 Alias。
- 云同步 Alias。
- 插件提供的 Alias。

## Comments

2026-09-13 实现记录：

- 新增 `search_alias_targets` IPC：输入目标名称按规范化名/显示名/拼音返回
  索引候选 Top N（默认 8，最多 20），不把完整索引发给 React。
- `user_aliases` 表新增 `target_id` 列（启动时自动 ALTER 迁移，旧库无感）；
  新保存的 Alias 写入稳定 AppItem id + 规范显示名。
- Rust 保存前校验：带 id 时必须能在索引中找到，否则报「目标应用不在索引中」；
  仅传名称（旧路径）须精确唯一命中，重名时明确报错要求从候选选择，
  不再静默绑到错误目标。
- 匹配语义升级：有 target_id 的 Alias 按 id 精确点名；旧行（无 id）仍按
  名称包含匹配，继续可用；编辑旧 Alias 时自动升级为 id 绑定。
- 设置页「别名」改为 AliasSettings 组件：目标输入框带候选下拉
  （图标 + 显示名 + 来源），未选择候选时「添加」按钮禁用；
  别名列表为旧数据显示「旧」角标。
- Alias 新增 / 修改 / 删除后均清空搜索缓存，关闭设置页即刷新当前 Query。
- 验证：`cargo test` 127 通过（含迁移、target_id 读写、重名拒绝用例）；
  `npm run build` 通过。待人工实机确认普通应用 / UWP / 同名候选行为。
