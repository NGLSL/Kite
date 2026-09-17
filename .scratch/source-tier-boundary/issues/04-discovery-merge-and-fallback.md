# 04: App Paths/Uninstall 归并与兜底

**What to build:** 通过启发式的 App Paths/Uninstall 条目：能与 Start Menu/Desktop/UWP 等正式入口确认同一产品或同一 launch identity 时，只补充 alias、exe 名、图标或 target，不新增独立行；无正式入口且明显是用户应用时，才作为 Tier C 兜底独立行（可搜）。Uninstall 优先服务名称/身份，而不是制造卸载向重复行。

**Blocked by:** 03 App Paths/Uninstall 启发式过滤

**Status:** resolved

- [x] 同产品/同 launch identity 的 App Paths 被正式入口吸收，展示仍是一行友好名
- [x] 被吸收的 exe 名仍可作为搜索关键词召回同一正式应用
- [x] 无正式入口且通过启发式的 App Paths 可作兜底独立行被搜到
- [x] Uninstall 能归并时不成独立重复行；不能归并时走与 App Paths 相同兜底约束
- [x] 归并测试与扫描/检索相关测试通过
- [x] Windows 实机抽查：无开始菜单快捷方式的已注册应用仍可搜；明显 helper 不出现（stability-v0.2.10 票 04）
- [x] `cargo check` / `cargo test` 通过
