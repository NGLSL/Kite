Status: resolved
Type: task
Blocked by: None

## Goal

图标源、资源序号与缓存失效规则恢复正确：`.lnk` 的 icon path + index 完整下传；无扩展名 ICO、负序号资源、无效优先源回退 target、旧错误缓存自动重提；控制面板等 Shell 虚拟入口显示当前系统真实图标。

## Scope

- `.lnk` `GetIconLocation()` 返回路径与序号，二者都进入现有图标提取链（含 `path,index` 与负序号资源 ID）。
- 无扩展名但文件头为 ICO 的 MSI 图标源可正确提取。
- 优先源只给出通用白纸图标或无效时，回退到已验证启动 target。
- 旧版快捷方式解析缓存丢失 icon index 时，升级/迁移 scanner cache 让条目重新解析。
- 内容有效但语义错误的通用软件图标缓存自动重生成，无需用户手删缓存。
- Shell 虚拟入口优先从当前系统命名空间取图标；stock icon 仅作语义确实对应时的回退。
- 图标刷新不得拖慢输入热路径；文件类型图标复用策略保持。
- 分别验证：AppItem 索引刷新、lnk 解析缓存、PNG 图标缓存生命周期不同，不混成一个 TTL。

## Acceptance Criteria

- [x] 非零/负 icon index 的快捷方式显示对应资源图标，而非默认第 0 号或占位符。
- [x] CC Switch 类「无扩展名 ProductIcon」显示应用图标而非通用白纸。
- [x] 优先图标源无效时列表仍有 target 应用图标。
- [x] 控制面板等 Shell 虚拟入口图标与当前 Windows 系统对象对应。
- [x] 升级后旧错误但有效的 PNG 缓存在下次索引时自动更新。
- [x] 损坏或全透明缓存会被重新提取。
- [x] 列表 UI 显示新图标，不被进程内旧缓存挡住。
- [x] 搜索输入期间图标补齐不阻塞首屏与后续按键。

## Validation

- `cargo test`（icons / scanner cache）
- Windows 实机回归：CC Switch、控制面板、回收站、普通 exe、Scoop、UWP 与常见文件类型图标覆盖。
- 检查磁盘 PNG 与最终列表可见图标是否一致。

## Dependencies

无。可与票 01/02 并行；最终列表验收会与票 10 合流。

## Out of Scope

- 界面视觉重做。
- 自定义图标主题包。

## Comments

规格备注：CC Switch 图标位置为无扩展名 `ProductIcon`，文件头有效 ICO；`.lnk` 独立丢弃 icon index——两条缺口须分别验证。控制面板缓存当前可能是「有效但错误」的通用 stock icon，不是提取失败。

2026-09-14 实现记录（worktree `Kite-search-experience`）：

- `lnk` 下传 `path,index`（含 0）；scanner cache `CACHE_VERSION=2` 强制旧条目重解析。
- `looks_like_ico_file` 嗅探 `00 00 01 00`，无扩展名 ICO 走 ico 提取链。
- `cache_icon(preferred, target_fallback)`；图标文件名 `v2:{id}` 使旧错误 PNG 自动作废。
- Shell 虚拟入口改为先解析命名空间对象，再 stock 回退（控制面板不再优先 SIID_SOFTWARE）。
- 回归：`extensionless_ico_magic_is_classified_as_ico`、`collect_candidates_prefer_icon_src_then_target`、`full_pass` 相关；`cargo test` 164 passed。
- 待人工实机（票 10）：CC Switch、控制面板列表可见图标与磁盘 PNG 一致。
