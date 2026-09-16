# 01: Portable 升为 Tier A 正式入口

**What to build:** 用户在设置里显式添加的 Portable 目录，索引来源按正式应用入口处理：空 Query 列表与 Start Menu/Desktop 同级展示，非空 Query 不按 Supplemental/Discovery 弱化。CONTEXT 领域词 SourceLayer 补齐 Tier A/B/C 语义，并写明 Portable 属 Formal。

**Blocked by:** None (can start immediately)

**Status:** implemented

- [x] `source_layer("portable")` 为 Formal（Tier A）；未知来源默认 Supplemental
- [x] 空 Query `order_by_recent` 中 portable 与 start-menu/desktop 一样进入正式补满段
- [x] 非空/短 Query 不因 portable 来源被沉底或隐藏
- [x] 相关回归测试通过（空 Query、分类映射）
- [x] CONTEXT.md 中 SourceLayer 词条描述与 Tier 一致，且 Portable 归 Formal
- [x] `cargo check` / `cargo test` 通过
