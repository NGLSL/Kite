# 05: Discovery 层展示与降噪

**What to build:** 对未吸收、仅作兜底的 Tier C 行（App Paths/Uninstall/命令）统一展示策略：空 Query 默认不刷进补满列表（Pin/最近使用仍覆盖）；非空短 Query 或弱匹配层沉底，不压过 Formal 应用；精确/明确匹配保护不变，`7z` 等仍可启动。体感：输 `7` 出 7-Zip File Manager，输 `7z` 命令仍明显。

**Blocked by:** 04 App Paths/Uninstall 归并与兜底

**Status:** implemented

- [x] 空 Query：未吸收的 app-paths/uninstall/commands/scoop 不进默认补满段
- [x] 空 Query：Pin 或最近使用的 Tier C 仍显示且可启动
- [x] 短 Query（如 `7`）：Formal 应用优先，弱匹配命令/发现行不霸占头部
- [x] 精确命令 Query（如 `7z`）：命令入口仍召回且可启动
- [x] `quality_tier <= PROTECTED_TIER_MAX` 不因本层降噪后移
- [x] 空 Query 与非空排序回归测试通过
- [ ] Windows 实机：空列表干净；`7` / `7z` 行为符合上文
- [x] `cargo check` / `cargo test` 通过
