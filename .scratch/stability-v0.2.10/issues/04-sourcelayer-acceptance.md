# 04: SourceLayer 实机验收收口

**What to build:** 在真实 Windows 上完成 source-tier-boundary 未勾的三项验收并留下证据；验收中发现的实现缺陷另开票，本票不顺手大改算法或分层。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] 02 Bootstrap：Cold Start 有图标、无注册表杂讯 —— 用户删除 index-snapshot 后冷启动正常
- [x] 04 Discovery：无开始菜单快捷方式的已注册应用仍可搜；明显 helper 不出现 —— 用户确认「目前看着也正常」
- [x] 05 Display：空列表干净；`7` Formal 优先；`7z` 命令仍可用 —— 用户确认正常；排序为通用 SourceLayer/保护层规则，无 `7z` 特判
- [x] 结果同步 source-tier 02/04/05；全绿后标 resolved
- [x] 未发现需单开的缺陷
- [x] 既有扫描/排序单测无回归（`cargo test -- --test-threads=1` 435 passed）

## Comments

- 2026-09：本机三项验收通过。`7z` 仅作测试样本与内置产品别名，排序走 CommandAlias 降噪 + PROTECTED_TIER 保护。
