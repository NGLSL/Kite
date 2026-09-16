Status: ready-for-agent
Type: enhancement

## Problem Statement

Kite 已把搜索热路径收住：唤起 P50 约 3ms、稳定内存约 56MB。但用户实际体感的最大缺口已经变成「启动后多久真正有东西可搜」。

当前启动链路是空 `AppIndex` → 后台直接 `ScanPass::Full` → metadata / UWP / 图标 / protocol / RetrievalIndex 全部完成 → 最后才一次性 publish。Full 全部完成之前，第一次启动没有可搜索索引。更早版本本有「快扫首屏 → 后台补齐」能力；为保证快照原子一致性，生产路径又收成了 Full-only，首屏可用性因此退化。

与此同时，索引把 Scoop shims、WindowsApps、WinGet Links、Chocolatey bin 等命令入口当成一等 `AppItem`。截图里 `7-Zip File Manager` / `7z` / `7zfm` / `7zg` 一窝蜂出现，说明 GUI 启动器被 PATH/包管理器命令噪声污染。用户真正会直接搜的 `winget` / `gcc` / `python` / `node` 仍需要可召回，但不能靠「删掉 Scoop 扫描」或「硬过滤 System32」这种会误杀正常入口的做法。

UI 侧还有一批明确、可快速收口的问题：底部快捷键文案硬编码 `Alt + Space`，与设置修改后的 `hotkey_label` 不同步；唤起时窗口只在创建时定位一次，多显示器下不会跟到鼠标所在屏；结果行不显示 target 路径；Scrollable 右侧没有给 scrollbar 留 gutter，会挡到 Alt+N 等内容；右键打开 ContextMenu 时顶层 widget 从 Container 变成 Stack，高度怀疑导致 Scrollable 状态重置、列表滚回顶部。

性能脚本目前只记录唤起延迟和稳定内存，缺少 `startup → first searchable` 与 `startup → full index`，因此会出现「P50 唤起 3ms 很漂亮、首次却等索引数秒」的指标盲区。

本轮不再动搜索算法本身，集中解决：**一启动就可搜、结果分层降噪、UI 收口、性能基线补指标**。

## Solution

正式启动模型定为 **Cold Start / Warm Start**，不再围绕「Fast→Full 是否实时替换」反复改架构。

```text
             Kite 启动
                 │
        snapshot 是否有效？
          /              \
        No                Yes
        │                  │
   Cold Start          Warm Start
        │                  │
快速扫核心来源        直接读完整缓存
        │                  │
图标 + 搜索可用       图标立即显示
        │                  │
        └──────┬───────────┘
               │
         用户已经可使用
               │
         Background Full
               │
          比较实际变化
          /          \
      无变化          有变化
        │              │
      不操作        写新 snapshot
                       │
              Launcher 隐藏后 / 下次打开
              再采用（可见时不打断）
```

### Cold Start（无 snapshot / 损坏 / schema 不兼容）

1. 快扫高价值正式入口：Start Menu + Desktop + App Paths + 内置系统入口。
2. 解析 `.lnk`，优先复用图标缓存或快速提取首批图标。
3. 附着 search fields → RetrievalIndex → **第一次可用**。
4. 后台再补 UWP / Uninstall / Scoop / commands / metadata / protocols / 深层来源。

目标指标：**首次安装到「有图标且可搜索」的时间**，不是 Full Scan 总耗时。

Bootstrap 不得原样复用当前 `ScanPass::Fast`：Fast 的扫描顺序把 Scoop / commands / uninstall 排在 Start Menu 前面，在 1.5 秒预算里优先扫了低价值来源。Bootstrap 必须限定为高价值正式入口来源；Cold Start 允许在首屏后补图标，但应尽快让首屏有辨识度。

### Warm Start（正常路径，绝大多数启动）

1. 读取 last-good snapshot（含已算好的规范化名、拼音、keywords、icon path 等）。
2. UI 立即可展示完整图标与列表。
3. 后台 `RetrievalIndex::build` 后即可搜索（不重复 .lnk COM / metadata / 图标提取 / 拼音）。
4. 同时后台 Full：与 snapshot 比较；无变化不操作；有变化写新 snapshot。
5. **Full 完成不立刻打断当前界面**：Launcher 可见时新 snapshot 先挂起，隐藏后或下一次打开再采用。

### Snapshot 内容

不缓存整个 `RetrievalIndex`，缓存扫描阶段已算好的条目：

```text
CachedAppSnapshot
├─ snapshot_version / search_schema_version
├─ id
├─ name / display_name
├─ target / args / working_dir
├─ source / is_lnk
├─ icon path / icon_src
├─ search_keywords / search_context
├─ normalized_name / normalized_display
├─ pinyin / pinyin_initials
```

搜索字段算法变更时提升 `search_schema_version`，旧缓存直接废弃重建，不做复杂迁移。

命令来源分层，而不是删目录：

| 来源层 | 内容 | 产品定位 |
| --- | --- | --- |
| 正式应用入口 | Start Menu / Desktop / UWP | 主结果 |
| 补充发现来源 | App Paths / Uninstall | 可搜、适度靠后 |
| 命令 Alias 来源 | Scoop shims / WindowsApps / WinGet Links / Chocolatey | 服务召回，默认不独立占首屏 |

结果行为：

- 空 Query：不显示 commands / scoop 裸命令，除非 Pin 或最近使用。
- 非空 Query：精确搜 `7z` 可以出来；短 Query / 模糊搜索不得让 `7z`/`7zfm`/`7zg` 一窝蜂顶上来。
- 已有正式 `7-Zip File Manager` 时，`7zfm`/`7zg` 优先作为搜索 Alias 帮它召回，而不是再显示成独立应用行。

UI 一批收掉：每次唤起按鼠标所在 monitor 重新定位；hover/selected 显示 target 路径第二行（行高仍 52px）；scrollbar 留 gutter；footer 绑定 `state.hotkey_label`；根 widget 始终是 Stack，右键不再改树结构。

性能脚本补 `time_to_window_ready` / `time_to_first_searchable` / `time_to_full_index` / `post_index_settle_cpu`。

## User Stories

1. 作为 Kite 用户，我希望第一次安装后一打开就能搜到开始菜单和桌面上的绝大多数软件，以便不用盯着空列表等完整扫描。
2. 作为已经用过 Kite 的用户，我希望升级或重启后 last-good 索引立即可搜，以便唤起体验与上次关闭时一样快。
3. 作为 Kite 用户，我希望首屏可搜索结果出现后，后台 Full 再慢也感觉不到，只要列表最终一次切到完整索引。
4. 作为正在输入的用户，我希望 Full 原子替换时当前 Query 自动刷新，且列表不短暂清空、不来回跳动多次。
5. 作为 Kite 用户，我希望启动期间安装器或目录变化不会让初始 Full 后再连跑两遍完整扫描，以便 CPU 和磁盘不白烧。
6. 作为维护者，我希望初始构建与 watcher 的首次脏事件有时序约定，以便排查「pending entry change; rebuilding again」这类日志。
7. 作为 Scoop 用户，我希望 `winget`、`gcc`、`python`、`node` 这类真会直接敲的命令仍能搜到，以便高级用法不丢。
8. 作为普通用户，我希望空 Query 时首屏是正式应用，而不是一串 shims 和裸命令，以便 GUI 启动器不像 PATH 浏览器。
9. 作为搜索 `7-Zip` 的用户，我希望模糊或短输入时 `7-Zip File Manager` 稳定靠前，而不是被 `7z`/`7zfm`/`7zg` 刷屏。
10. 作为精确输入 `7z` 的用户，我仍希望该命令入口可召回并可启动，以便命令语义不被分层一刀切掉。
11. 作为已有正式入口的用户，我希望对应命令别名优先作为召回证据归并到正式 AppItem，而不是再多一行几乎一样的结果。
12. 作为被 Pin 或最近使用过的命令入口用户，我希望它在空 Query 仍可见，以便个人习惯覆盖默认降噪。
13. 作为 Kite 用户，我希望命令降权不删除索引项、不改启动 target，以便误伤可恢复。
14. 作为多显示器用户，我希望鼠标在哪块屏按快捷键，面板就出现在哪块屏工作区，以便和 uTools 等启动器一致。
15. 作为 Kite 用户，我希望每次唤起都重新计算 monitor 位置，而不是沿用窗口创建时那块屏。
16. 作为浏览结果的用户，我希望 hover 或选中时能看到该条的 target 路径，以便区分同名或相近入口。
17. 作为 Kite 用户，我希望路径副标题很弱、不改变行高，以便列表密度和滚动选中定位不被打乱。
18. 作为键盘用户，我希望 scrollbar 不再盖住列表右侧内容，以便 Alt+N 等快捷键提示仍可点到、看得到。
19. 作为用过右键菜单的用户，我希望打开菜单后列表仍停在原滚动位置，而不是弹回顶部。
20. 作为修改过全局热键的用户，我希望底部快捷键文案始终显示当前生效热键，而不是永远写着 Alt + Space。
21. 作为维护者，我希望托盘提示里的热键文案与真实热键一致，避免同一应用两处说法不同。
22. 作为维护者，我希望性能基线能分别报出窗口就绪、首次可搜索、完整索引完成，以便再也不会只优化唤起延迟。
23. 作为维护者，我希望 Full 完成后还有一段 settle CPU 观察，以便确认索引稳定后没有后台空转。
24. 作为 Kite 用户，我希望 Bootstrap 阶段的图标可以先是占位或缓存旧图，Full 完成后再一次换成完整图标，以便首屏速度优先。
25. 作为 Kite 用户，我希望单条坏快捷方式、不可读目录或某个命令目录失败，不影响 Bootstrap 和 Full 的其余 AppItem。
26. 作为通过 App Paths 注册程序的用户，我希望 Bootstrap 就能搜到无开始菜单快捷方式的程序，以便补充发现来源不拖到 Full 才出现。
27. 作为系统工具用户，我希望内置系统入口在 Bootstrap 阶段也可搜，以便回收站、控制面板等常见入口不缺席首屏。
28. 作为 Kite 用户，我希望 Full 阶段才引入 Scoop/commands/UWP/metadata/protocol，且只替换一次，以便中间态不会反复改列表。
29. 作为托盘用户，我希望「重新扫描应用」仍走完整 Full 并原子替换，以便手动恢复路径不被 Bootstrap 短路。
30. 作为维护者，我希望来源分层是索引与结果策略的概念分层，而不是按品牌写 7-Zip 特例，以便新安装软件同样受益。
31. 作为 Kite 用户，我希望明确匹配保护、Pin、用户 Alias 在降噪后仍优先于命令噪声，以便排序规则可预测。
32. 作为维护者，我希望本轮不继续改 matcher/ranker 热路径，以便已稳定的搜索延迟不被顺手重构破坏。
33. 作为 Windows 实机验收者，我希望多显示器定位、右键滚动、footer 热键、hover 路径都有可见验收点，以便编译通过不算完成。
34. 作为 Kite 用户，我希望 last-good 快照失效或版本不兼容时安全回退到 Bootstrap+Full，而不是启动失败。
35. 作为维护者，我希望 last-good 与当前代码索引 schema/代际兼容规则明确，避免旧快照静默给出错误 target。

## Implementation Decisions

- 本轮不动搜索召回与排序热路径本身；只新增/调整索引构建阶段、来源分层标记、结果可见性策略、UI 展示与性能度量。
- 启动采用 **两阶段，而不是恢复历史四阶段**（Fast → 补图标重建 → UWP 重建 → Full 再重建）。前台最多一次「Bootstrap/last-good → Full」快照替换。
- **Bootstrap Pass** 为新的有预算扫描阶段：只扫 Start Menu、Desktop、App Paths、内置系统入口；构建 search fields 与 RetrievalIndex 后立即 publish。明确排除图标提取、版本 metadata、UWP、uninstall、Scoop shims、commands 目录、protocol merge。
- Bootstrap **不得**直接复用现有 `ScanPass::Fast` 的来源集合与扫描顺序；Fast 继续留给测试或废弃，不得作为生产首屏路径。
- **Full Pass** 保持完整来源与 enrichment（metadata、UWP、图标、protocol、全量 RetrievalIndex），完成后原子替换，并写入 **last-good snapshot**。
- last-good 只在 Full 成功完成后更新；载入时校验版本/schema/代际，失败则丢弃并走 Bootstrap。
- 同一时刻仍最多一次在跑的索引构建（single-flight）；Bootstrap 与 Full 共享该约束，Full 可在 Bootstrap publish 之后启动，但不得在 Full 未完成时发布不完整快照。
- Watcher 与初始构建的时序：初始 Bootstrap/Full 发布完成前到达的入口脏事件只记 pending，不在首屏构建进行中叠跑第二轮 Full；首屏发布后再消费 pending。
- 命令入口在 `AppItem` 上需要可区分的来源层级：正式入口 / 补充发现 / 命令 Alias。该层级用于结果可见性与归并，不表示删除这些来源的索引。
- 不硬过滤 `C:\Windows\System32` 等路径前缀；App Paths / Builtin 系统入口仍可进入补充层或正式系统入口层。
- 空 Query 结果策略：默认隐藏命令 Alias 层裸命令；Pin 或 Recency 可覆盖。
- 非空 Query：精确/高质量匹配仍可展示命令入口；短 Query、松散模糊匹配对命令 Alias 层降可见或归并到同族正式 AppItem。
- 当正式 AppItem 与命令入口共享启动语义或同名/同 stem 时，命令名优先作为召回别名服务正式入口，而不是再占用独立首屏行；不能确认同一启动语义时保留可启动结果，避免错误合并。
- 用户 Pin、用户 Alias、明确匹配保护继续优先于命令降噪；Demote 语义保持可撤销，不删索引、不改 target。
- 多显示器：每次唤起用当前光标点选 monitor，按该 monitor 的工作区水平居中、垂直约 1/3 定位窗口，再 show/focus。使用 Win32 光标与 monitor API，不靠创建窗口时的静态坐标。
- 结果行在 hover 或 selected 时显示 target 路径第二行：小字号、灰色、超长省略；**行高保持不变**，禁止因 hover 动态增高。
- 结果列表为 scrollbar 预留右侧 gutter；Alt+N 等行内操作区不得伸入 scrollbar 下方。
- 根视图始终为 Stack：主内容 + overlay 槽。无菜单时 overlay 为空，有 ContextMenu 时替换 overlay 内容；不得在开/关菜单时改变 Scrollable 在 widget 树中的位置。
- Footer 快捷键文案绑定当前热键 label；设置修改成功后 footer 与 tray 提示同步，不再字面量写死默认热键。
- 性能基线脚本新增并汇总：`time_to_window_ready`、`time_to_first_searchable`、`time_to_full_index`、`post_index_settle_cpu`。既有唤起延迟与内存指标保留，用于防回归。
- 持久化边界：last-good 是索引快照缓存，不是新的搜索数据库；历史仍用现有本地存储决策；不引入云服务或遥测。
- 模块职责保持：扫描/索引、检索与结果策略、UI、性能脚本各管各的；UI 只消费快照并触发刷新，不承担来源分层判定。

## Testing Decisions

好测试只断言外部可见行为：可搜索时机、列表中的名称/顺序/是否独立成行、target 是否仍可启动、footer 文案、路径是否显示、窗口所在 monitor、性能脚本输出的阶段耗时。不断言内部函数调用次数、锁实现或具体分数常量。

三类主缝：

1. **索引发布时间线**（最高优先）
   - 覆盖：冷启动无 last-good 时先发布 Bootstrap 可搜索快照；Full 完成后只原子替换一次；有 last-good 时跳过空窗直接可搜索；last-good 损坏时回退 Bootstrap；初始构建期间入口脏事件不叠跑第二轮 Full。
   - 先验：既有 scanner / backend single-flight 与 Full 原子发布测试；历史 `search-experience-next` 票 01 的快扫→Full 思路，但期望值按本轮两阶段语义重写。
   - 观察：第一个可搜索快照是否含高价值正式入口；替换次数；Full 完成后是否含 UWP/命令等补充来源。

2. **结果策略**
   - 覆盖：空 Query 隐藏命令裸行（Pin/最近使用除外）；精确 `7z` 可召回；短/模糊 Query 不被 `7z`/`7zfm`/`7zg` 刷屏；有正式 7-Zip 入口时命令名作为 alias 召回并归并；用户 Pin/Alias/明确匹配保护不被降噪压制；不按路径前缀硬杀 System32。
   - 先验：现有检索 fixture 与搜索样本评估方式；来源分层只作为文档字段/策略输入，不写产品名分支。
   - 样本用稳定 fixture 身份，不只断言显示名。

3. **视图态**
   - 覆盖：footer 使用 state 中的当前热键 label；结果行 selected/hover 时渲染路径副标题且行高不变；根 Stack 始终存在（菜单开关不改变列表 widget 位置这一外部结构约定）。
   - 先验：现有 UI 消息/状态测试若有则复用；Iced 难测的布局细节以「结构约定 + 实机」组合验收，不在单测里抠像素。

多显示器、真实 scrollbar 视觉遮挡、右键滚动位置、托盘文案、性能脚本阶段指标：Windows 实机验收。编译通过不算完成。

命令：
- `cargo check`
- `cargo test`
- `cargo build --release`
- 扩展后的 `scripts/measure-performance.ps1`（或等价脚本）产出阶段指标
- 实机：冷启动首搜时间、多显示器跟随、右键不回顶、footer 热键、hover 路径、`7z` 族结果

## Out of Scope

- 继续优化 matcher/ranker 热路径、改 MatchScore/FinalScore 算法、重写拼音或模糊匹配。
- 恢复历史四阶段「多次中间快照」流水线。
- 直接删除 Scoop / WindowsApps / WinGet / Chocolatey 扫描来源。
- 按 `System32` 等路径前缀硬过滤入口。
- 把 PATH 上全部 exe 都当应用入口。
- UWP/Store 安装事件实时监听、固定间隔全盘重扫。
- 云同步、遥测、插件系统、跨平台、换 UI 框架、视觉大改版。
- 自建文件索引或替换 Everything。
- 为 7-Zip 或任意单一产品名写扫描/排序特例。
- 签名、版本号、推送、合并和发布流程。

## Further Notes

- 代码现状（dev `44c3b3c` 附近）：生产 `request_build` 只跑 `ScanPass::Full`；`ScanPass::Fast` 仅测试调用；无 last-good AppIndex 载入；watcher 与初始 Full 同时启动，`PENDING_REBUILD` 可导致再跑一轮 Full。
- 历史规格 `search-experience-next` 曾验收「快扫首屏 + 后台 Full」，与当前 Full-only 生产路径不一致。本轮以当前代码为准，明确回归两阶段首屏，但禁止回到多次可见中间重建。
- Bootstrap 来源选择是产品决策：1.5s 预算优先 Start Menu/Desktop/App Paths/系统入口，才能让「一打开就有绝大多数软件」成立。
- 命令分层同时服务结果降噪与 Bootstrap 扫描量下降；Full 仍索引命令入口，保证精确召回与升级兼容。
- 优先级：P0 首次可搜索（含 watcher 时序与 last-good）→ P1 命令/Scoop 结果分层 → P2 UI 收口（多显示器、右键滚动、scrollbar、动态热键、hover 路径）→ 性能指标与基线随 P0/P2 验收落地。
- 若只选一件事，先做首次可搜索索引；它是最明显体验缺口，且根因已清楚。
- 后续拆票应保持上述顺序；实现完成后统一走 `/code-review`（Standards + Spec 双轴）。
