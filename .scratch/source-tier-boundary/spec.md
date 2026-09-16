Status: ready-for-agent
Type: enhancement

## Problem Statement

上一批已做出 SourceLayer、空 Query 隐藏命令、弱匹配命令降噪，以及 Bootstrap 首屏。用户实机仍感到「技术上能启动、但不像应用」的结果在污染列表。

根因是产品边界仍偏「可执行文件索引器」：

1. **App Paths** 只要求注册表默认值指向现存 exe 就生成独立 `AppItem`。helper、updater、SDK/runtime 端点会和 Chrome、微信一样进结果。
2. **Uninstall 注册表** 同样物化为独立可展示行，而不是主要用来补名称、安装根和身份。
3. **Portable** 被当成 Supplemental，但用户在设置里显式添加的目录应属正式应用入口。
4. **Bootstrap** 仍扫 App Paths，冷启动首屏就可能混进注册表 EXE，而不是只给「用户认为是应用」的 A+B。
5. 命令入口（Scoop/WindowsApps/WinGet/Chocolatey）已有降噪，但 App Paths/Uninstall 还没有同等「默认不当正式应用」的约束。

Kite 是 Launcher，不是 executable indexer。本批把三层来源边界定死，避免以后每加一个 Windows 来源就再脏一次。

## Solution

正式产品分层（与 CONTEXT 中 SourceLayer 对齐，补齐 Tier 语义）：

```text
Tier A — User-facing App
  Start Menu .lnk / Desktop .lnk / UWP·AppsFolder / 用户 Portable
  → 正常展示与排序

Tier B — Curated System
  Kite 人工维护的系统工具 / Windows 设置（system_entries）
  → 正常展示；不靠扫 System32

Tier C — Discovery / Command
  App Paths / Uninstall / Scoop / WinGet Links / WindowsApps / Chocolatey
  → 补充搜索信息、Alias、图标或精确命令
  → 默认不当正式应用刷屏
```

行为规则：

1. **Portable 升为 Tier A**：与 Start/Desktop 同级进入正式入口与 Bootstrap 能力集合（用户已配置的目录）。
2. **App Paths 降为 Tier C**，收集时先分类：
   - 能与已有 Tier A/B 确认同一产品/同一 launch identity → 只作 alias、exe 名、图标或 target 补充，**不独立成行**。
   - 明显用户应用且当前无正式入口 → **兜底独立行**（可搜；非空 Query 弱于友好入口；短/弱匹配不刷屏）。
   - 系统组件 / helper / updater / SDK / runtime → **不展示**。
3. **Uninstall 降为 Tier C**：优先参与归并与名称补充；独立成行规则与 App Paths 兜底一致。
4. **命令入口**维持：空 Query 默认隐藏（Pin/最近仍可覆盖）；非空弱匹配沉底；精确/明确匹配保护仍可启动（如 `7z`）。
5. **Bootstrap = 仅 A + B 高价值来源**：Start Menu、Desktop、（若已配置）Portable、内置系统入口。**不含** App Paths、Uninstall、Scoop、commands、UWP、metadata、protocol。**Bootstrap 会批量提图标**（Cold Start 第一印象优先于省 200–300ms；Warm Start 走 snapshot 不付这笔成本）。Full 再补齐 C 与 UWP。
6. **System32**：继续不枚举普通 exe；系统功能只走 curated `system_entries`。

用户体感验收：

- 空 Query：干净的应用列表，无 bare command / 无系统 helper。
- 输 `7`：`7-Zip File Manager` 优先，不是 `7z`/`7zfm`/`7zg` 刷屏。
- 输 `7z`：命令入口仍明显可用。
- 没有开始菜单快捷方式、但用户常用的已注册应用：仍可通过 App Paths 兜底搜到。
- 冷启动首屏：只有用户认为是应用的条目。

## User Stories

1. As a launcher user, I want empty-query results to look like my installed apps, so that I am not browsing package-manager shims and registry helpers.
2. As a developer, I want to type `7` and see 7-Zip File Manager first, so that short queries surface products instead of command variants.
3. As a developer, I want to type `7z` and still launch the 7z command, so that exact command intent is never lost.
4. As a power user, I want apps I registered only via App Paths (no Start Menu shortcut) to remain searchable, so that obscure tools stay reachable.
5. As a power user, I want App Paths helpers like updaters/SDK endpoints hidden, so that registry noise does not look like installed apps.
6. As a user with portable apps, I want directories I explicitly added treated as first-class apps, so that they appear and rank like Start Menu entries.
7. As a user, I want Uninstall registry mainly to improve names/identity of existing apps, so that I do not get duplicate uninstall-oriented rows.
8. As a user on first install, I want Bootstrap to show Start/Desktop/Portable/system tools **with icons**, so that the first searchable index feels like a launcher, not a file dump.
9. As a user, I want Windows system features as curated friendly entries (Task Manager, Control Panel…), so that I never need raw System32 enumeration.
10. As a user who pinned or recently used a command alias, I want it still visible on empty query, so that my personal workflow is not broken by product layering.
11. As a user, I want formal apps to outrank discovery/command hits on weak or short matches, so that GUI apps stay primary.
12. As a user, I want unknown future sources to be treated as discovery (not silent formal apps), so that new scanners cannot accidentally flood the empty-query list.

## Implementation Decisions

- **词汇**：CONTEXT 的 SourceLayer 保留，补写 Tier A/B/C 语义；Portable 从 Supplemental 调整为 Formal（Tier A）。未知 source **不进** Formal，避免新扫描器误刷空列表。
- **分层 API**：`source_layer` / 空 Query 隐藏策略以 Tier 为准。**Formal 是白名单**（start-menu/desktop/uwp/apps-folder/portable）；**未知 source 默认 Supplemental**，避免新扫描器忘记分类就变成正式应用。空 Query 默认隐藏：CommandAlias + Supplemental（含未吸收 Discovery）。Pin/最近使用仍覆盖。
- **扫描编排**：
  - Bootstrap：Start Menu、Desktop、配置的 Portable、curated system；不再调用 App Paths/Uninstall/Scoop/commands。
  - Full：在现有来源上增加 App Paths/Uninstall 的 **Tier C 后处理**（分类 + 归并 + 兜底），而不是无条件 `raw.extend`。
- **App Paths 分类启发式**（可单测；实机再调）：
  - helper 拒绝：复用/扩展命令名 helper 标记（uninstall/updater/setup/helper/crashpad/…）；路径位于 System32、WindowsApps、WinGet Links、Chocolatey bin 等系统/包管理目录时拒绝独立展示。
  - 同产品吸收：与已有正式入口同一 `launch_identity` / install root / exe stem 与展示名族时，只合并 keywords/icon/target，不新增行。
  - 兜底独立：其余现存 exe 生成 Tier C 行，参与检索但弱于 Formal。
- **Uninstall**：同一策略——能归并则归并；不能归并且通过启发式才兜底；否则不展示。
- **排序**：扩展既有「友好入口折扣 / 命令弱匹配降噪」到未吸收的 App Paths/Uninstall：短 Query 与弱匹配层沉底；`quality_tier <= PROTECTED_TIER_MAX` 不因分层后移。
- **归并主入口**：launch representative 优先级保持 Start/Desktop lnk > Portable >（兜底）App Paths > commands > uninstall。
- **不**递归扫 System32；**不**删除索引中的命令能力；**不**改搜索热路径算法本身（只动分层与展示策略）。
- **性能**：Bootstrap 变窄应只降冷启动噪声与耗时，不引入新的 Full 阻塞；不为此批新增必测 CPU 指标（沿用上一批脚本）。

## Testing Decisions

只测外部行为，不测私有函数内部实现。优先四条已有高层缝：

1. **空 Query 列表**（`order_by_recent`）  
   - bare `commands`/`scoop`/未吸收 App Paths 不进补满段  
   - Pin/最近的 Tier C 仍出现  
   - Portable 进正式列表  
   - 先例：`empty_query_hides_bare_command_aliases` 等

2. **非空 / 短 Query 排序**（`search_with_personalization` / `RetrievalIndex`）  
   - `7` → Formal 7-Zip 优先于 `7z`/`7zfm`  
   - `7z` 精确仍可召回命令行  
   - 弱匹配 App Paths 不压过 Formal  
   - 先例：`short_query_prefers_formal_app_over_command_shims`、`source_preference_*`

3. **扫描库存**（`scan_apps_pass_with_options`）  
   - Bootstrap 不含 app-paths/uninstall/scoop/commands  
   - Bootstrap 含配置的 portable  
   - Full 在 helper 路径上不产生独立 App Paths 行  
   - 先例：`bootstrap_pass_skips_scoop_and_command_sources`

4. **归并主入口**（RetrievalIndex launch collapse / merge）  
   - App Paths exe 与 Start Menu 友好名同产品 → 单行，exe 名可搜  
   - 先例：`same_launch_identity_displays_one_friendly_row`、WPS/微信归并测试

启发式纯函数（helper 标记、系统路径拒绝）可加小单测。真实 HKLM App Paths 清单、多显示器等不在本批自动化范围。

## Out of Scope

- 重写搜索算法、拼音、模糊或 MatchScore 体系
- 枚举 System32 / 整棵 PATH
- 用户自定义 Alias UI、Demote UI 增强
- 托盘热键文案、7z stem 归并遗留项（另批）
- 安装器 / 更新通道
- 把 App Paths 完整「用户应用」分类做成 ML 或复杂 PE 元数据；本批只启发式 + 实机验收

## Further Notes

- **Snapshot Windows 覆盖写**：审查中发现的 P0，虽不在本 spec 原文，已同批修掉（delete-then-rename）。
- 上一批 SourceLayer 已是雏形；本批是**收紧**而不是推翻。优先改分类与收集策略，少动 retrieval 热路径。
- 保护层规则不变：明确匹配（Name Exact 等）不因命令/发现层降噪被压到弱匹配之下。
- 代码里 `include_supplemental_sources` 与 App Paths 收集目前不在同一闸门；实现时要显式拆开 Bootstrap 与 Full 的 App Paths 行为，避免「以为 Bootstrap 没扫、实际仍扫了」。
- 验收以 Windows 实机为准：编译与 400+ 单测通过 ≠ 完成。
