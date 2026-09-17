# 01: Prefactor Expand — ResultSource + ResultAction

**What to build:** 在现有搜索结果模型旁并行引入通用的 ResultSource 与 ResultAction（覆盖 App/File/Web/Builtin 的展示与行为边界，预留 Plugin 变体）。用户侧搜索、启动、文件与网页结果行为与现在完全一致；这是「先让改动变容易」的 expand，不交付新的插件能力。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] 定义 ResultSource（含 App/File/Web/Builtin，以及 Plugin { plugin_id, provider_id } 的类型形状）
- [x] 定义 ResultAction（LaunchApp / OpenFile / OpenUrl / CopyText / Plugin { plugin_id, action_id, payload }）
- [x] 既有结果构造路径可附着 source + action，而不改变排序、个性化、搜索代际语义
- [x] 应用搜索主路径与 History/Pin/Demote/明确匹配保护回归测试全部通过
- [x] 不删除、不改造 AppItem 索引与 MatchScore 排序逻辑（本票不做 contract）
- [x] `cargo test` 相关模块全绿（全量里 2 个 scanner 环境用例偶发，单独复跑通过，与本票无关）

## Notes

- 对应规格 Stage 0 的 expand 侧；全面去掉「结果内嵌 AppItem」的 contract 不在本票。
- 测试主缝：既有 search 统一入口 + `model::result_expand_tests`。
- 后续 02–09 都建立在本票类型之上。
- Search-result 字段 serde 键为 `result_source`，避免与 flatten 的 `AppItem.source`（扫描来源）冲突；领域关系已写入 `docs/CONTEXT.md`。

## Answer（dev 工作树）

- `src/model.rs`：引入 `ResultSource` / `ResultAction`，`SearchResult` 挂载 source/action；`scored`/`with_quality_tier` 推导默认值；提供 `with_source_action`。
- 物化路径 `with_quality_tier`：`src/search/retrieval/doc.rs`、`src/search/diagnose.rs`。
- 领域词汇：`docs/CONTEXT.md` 补充 ResultSource / ResultAction，并与 SourceLayer、AppItem.source 区分。
- 启动路径仍走 UI 内 item 分支（工单 02 再迁到 ResultAction）。
- 验证：`cargo check`；`result_expand` 8 测通过；search/history/ui.results 关键回归通过。
