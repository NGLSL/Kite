Status: ready-for-agent
Type: enhancement

## Problem Statement

v0.2.9 已完成搜索 worker、SourceLayer、Bootstrap/Warm Start、last-good 原子写等一批架构改动，本机搜索延迟已到亚毫秒～数毫秒量级，继续抠算法收益很低。当前风险不在「功能能不能跑」，而在**稳定性可见性与验收闭环**：

1. **索引快照已原子覆盖，其它 JSON 缓存没有。** LNK 解析缓存、UWP 枚举缓存、exe 元数据缓存仍是 `write tmp → remove old → rename`，失败静默。进程死在中间态时，下一轮会静默重解析大量 COM/枚举，用户只感到启动变慢，日志几乎看不出根因。
2. **Warm Start 可以吃任意旧快照。** schema 兼容即恢复，超过 24h 只记日志。这有利于启动体验，但若后台 Full 长期失败，用户可能连续数周用旧索引，设置页和日志都**看不出来**。
3. **SourceLayer 实机验收未收口。** 实现已落地并有单测，但 `.scratch/source-tier-boundary` 仍有三项 Windows 实机未勾：Cold 图标/注册表杂讯、无开始菜单的 App Paths 兜底与 helper 过滤、`7` / `7z` 排序。编译与单测通过 ≠ 完成。
4. **发布流程靠约定不靠闸门。** `ci.yml` 已在 push main / PR 上跑 test+release build；真正的问题是 tag 可能打在未绿的 main 上，Release 首步「Verify tag commit is on main」失败后才暴露（v0.2.9 曾 attempt=2）。

Kite 已从「堆功能」进入「别再大改底层、把索引健康和验收做实」的阶段。本批只收稳定性与可见性，**不**重开搜索算法。

## Solution

面向维护者与最终用户的最小闭环：

1. **统一 JSON 原子写**：索引快照、LNK 缓存、UWP 缓存、元数据缓存共用同一套 Windows 原子替换路径；失败写日志，不再 `let _ =` 吞掉。
2. **Full Index 健康可见**：记录 snapshot 年龄、最近 Full 成功/失败；日志与设置页只读区域能看出「正常 / 快照过旧 / 完整扫描失败」。**不**恢复 Warm 硬过期，**不**改启动策略。
3. **SourceLayer 实机验收收口**：按既有 02/04/05 清单在真实 Windows 上勾选；若发现缺陷，单开修复票，不在验收票里顺手大改。
4. **发布顺序固化**：文档与（可选）Release 侧检查固定「main 上 CI 绿 → 再打 tag」。不新建第二套 CI。

另有一项**版本外运维**（不进本 spec 验收）：手工编辑 v0.2.9 Release 正文，去掉已删除的 SignPath 政策链接，与 `08ee691` 对齐。

用户体感：

- 缓存损坏时日志能定位，而不是「莫名其妙变慢」。
- 设置页能看出索引是否健康，而不是一直假设 Full 在默默成功。
- 空列表/`7`/`7z`/冷启动首屏在真机上与 SourceLayer 规格一致。

## User Stories

1. As a maintainer, I want every index-related JSON cache written through one atomic replace path, so that a crash mid-save cannot leave a half-deleted cache that forces a silent full reparse.
2. As a maintainer, I want cache write failures logged with the destination path, so that I can diagnose “startup got slower after a crash” without guessing.
3. As a user, I want LNK/UWP/metadata caches to survive process kill during save the same way last-good already does, so that warm rebuilds stay fast.
4. As a user, I want corrupted or missing side caches to degrade to a rescan, not to a stuck or empty launcher.
5. As a maintainer, I want Full scan success and failure timestamps recorded, so that “Warm from a months-old snapshot” is a visible state, not a silent one.
6. As a user, I want the settings page to show a simple index health line (healthy / aged snapshot / recent Full failures), so that I know whether to hit 重新扫描 or report a bug.
7. As a user, I want aged Warm starts to keep working exactly as today, so that health visibility never makes cold start worse.
8. As a maintainer, I want continuous Full failures to accumulate a count in logs, so that permission or scanner bugs do not hide behind a single info line.
9. As a launcher user, I want cold start results to look like installed apps with icons and no registry helpers, so that first impression matches a launcher rather than an executable dump.
10. As a power user, I want apps that only have App Paths (no Start Menu shortcut) to remain searchable, so that obscure tools stay reachable.
11. As a user, I want obvious helpers/updaters/SDK endpoints from App Paths to stay out of default results, so that registry noise does not pollute search.
12. As a developer, I want typing `7` to prefer 7-Zip File Manager over command shims, and `7z` to still launch the command, so that short queries and exact commands both work.
13. As a maintainer, I want SourceLayer real-machine checklists closed with evidence, so that “implemented” means accepted on Windows, not only unit-tested.
14. As a releaser, I want the release process to require a green main before tagging, so that Release does not fail at the first verification step after the tag is already public.
15. As a releaser, I want the existing `ci.yml` to remain the only PR/main gate, so that we do not grow a second parallel CI.
16. As a user, I want v0.2.10 to be a boring upgrade: no settings migration, no search behavior change beyond fixes discovered in acceptance.
17. As a future contributor, I want one shared atomic JSON write helper so that the next cache does not re-implement remove-then-rename.
18. As a maintainer, I want MetaCache included in the atomic write unification, so that all four JSON side-files behave the same.
19. As a user, I want health display to be read-only and non-blocking, so that a bad Full never freezes the settings page.
20. As a product owner, I want plugin/AI/search-algorithm work explicitly deferred, so that this version stays a stability close-out.

## Implementation Decisions

- **词汇**：沿用 CONTEXT 的 AppItem、SourceLayer/Tier、搜索代际、last-good 快照。新增内部概念：**Index Health**（索引健康）= 最近 Full 成功/失败时间、失败原因摘要、当前 last-good 年龄、是否来自 aged Warm。Health **不是**用户 Demote/Alias，也不改变检索排序。
- **原子写公共入口**：抽出「写临时文件 → Windows 原子替换到目标路径」的公共函数；索引快照已有的 `ReplaceFileW` / `MoveFileExW` 路径作为唯一实现来源，其它调用方复用，不在多处复制 API 细节。
- **纳入范围的写盘**：index snapshot、LNK `ScanCache`、`UwpCache`、exe `MetaCache`。四者失败均写 warn 日志；缓存失败不阻断扫描（丢缓存只导致重解析）。
- **tmp 命名**：与目标同目录、可区分扩展名（沿用现有 `.json.tmp` 惯例），避免并行扫描互踩；不为此引入跨进程锁。
- **Index Health 存储**：小状态文件或现有 runtime data 旁的轻量 JSON/键值即可，schema 独立、损坏时按「未知健康」处理，不影响 Warm。
- **更新点**：Full 成功写 last-good 后记 success；Full 失败（扫描错误或 snapshot save 失败）记 failure + 简短原因；Warm load 时记录 snapshot age 与是否 aged。
- **设置页**：只读一行或一小段文案，格式化自 Index Health；不提供复杂操作，不阻塞 UI。文案随健康态变化（正常 / 快照过旧 N 小时 / 完整扫描失败 N 次）。
- **Warm 策略不变**：不恢复 24h 硬拒绝；aged 只影响日志与健康展示。
- **SourceLayer**：本批以实机验收与证据记录为主；验收中发现的实现缺陷另开票，避免验收票变成大杂烩。
- **发布流程**：维护者文档写明「push main → CI 绿 → tag → Release」；可选在 Release 校验步骤增加「tag commit 对应 CI success」。**不**新建 `ci.yml` 变体，**不**把 NSIS 挪到 PR。
- **版本**：目标 **v0.2.10**，主题 Stability + Search Quality Acceptance。版本号在实现完成后、打 tag 时落定。

## Testing Decisions

只测外部行为，不测私有函数内部实现。优先已有高层缝：

1. **原子写 / 缓存 roundtrip**（temp dir 上 save→load）
   - 覆盖已有目标文件时保存成功且再 load 内容正确（snapshot 已有先例：`second_save_overwrites_existing_snapshot`）
   - ScanCache / UwpCache / MetaCache 覆盖写后可 load（沿用各自现有测试并接到公共实现）
   - 失败路径：不 panic；公共入口在不可写目标上返回错误或由调用方记日志（按最终 API 形状断言外部可观测行为）
   - 先例：`src/app/snapshot.rs`、`src/app/scanner/cache.rs`、`src/app/scanner/metadata.rs` 中现有 save/load 测试

2. **Index Health 状态**（纯状态对象 + 更新点）
   - Full 成功 → success 时间刷新、失败计数清零或按约定重置
   - Full 失败 → failure 时间与计数增加，原因摘要可读
   - aged Warm load → 记录 age，**仍返回可用索引**（不拒绝）
   - 设置页展示：由 health 状态格式化出的字符串含关键信息（有无「过旧」「失败」等）；不测 iced 布局像素
   - 先例：`src/ui/backend.rs` 中 snapshot 发布测试、`src/ui/settings_view.rs` 的结构约定 + 实机组合

3. **SourceLayer 实机清单**
   - 自动化：沿用 source-tier 既有扫描/排序单测，本批不扩算法测例
   - 实机：issue 02/04/05 三项勾选；结果写入对应 issue Comments
   - 编译通过与单测通过 **≠** 验收完成

4. **发布流程**
   - 以文档与可选脚本/Action 步骤为准，不强制 Rust 单测
   - 若实现「tag 需 CI success」检查，用故意失败路径验证报错信息可读

## Out of Scope

- 重写搜索算法、拼音、模糊、MatchScore、再加 matcher
- 真实 Query 回归 corpus / 与 uTools 对拍（属 v0.3.0）
- 插件、计算器、AI、扩展能力
- 恢复 Warm 24h 硬过期或改 Bootstrap/Warm 启动策略
- 新建第二套 CI，或把 NSIS/Release 下放到每个 PR
- Authenticode 签名申请本身
- 手工编辑 GitHub Release 页面（运维项，不作为本批代码验收）
- 多显示器、托盘文案等已在其它 batch 的项

## Further Notes

- **CI 已存在**：`.github/workflows/ci.yml` 在 push main / PR 上跑 Windows `cargo test` + `cargo build --release`。本批只补发布顺序，不要「再加一个 CI」。
- **优先顺序建议**：原子写 → 健康态 → SourceLayer 实机（可并行）→ 发布流程。发 v0.2.10 前前三项应绿。
- **验收以 Windows 实机为准**：与 source-tier spec 一致，编译与单测通过不算完成。
- 手工修 v0.2.9 Release 的 SignPath 死链可随时做，与本批代码无关。
