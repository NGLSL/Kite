# 09: 真实名称长词元进一阶纠错

**Status:** resolved
**Blocked by:** None (can start immediately)
**Type:** bugfix

**What to build:** 应用名称里较长的英文词（13–18 字符）打错一个字母时，仍然能被找到，而不是静默搜不到。

- [ ] 真实名称中 13–18 字符的词元进入一阶删除变体（当前上限 12 会把这一段整体排除）
- [ ] 不恢复全量派生词生成；不扩大距离 2 的生成集合
- [ ] 删除索引的键数（内存水位约束）不显著回退
- [ ] 长词元单字母错拼的召回回归测试

## Notes

- 只让**真实名称词元**的 13–18 字符部分进入一阶删除，不放开派生词，也不放宽距离与长度上限。
- 上一轮的内存优化（种子筛选、长度限制）必须保留——本票只补这一段缺口，不是回退。
- compact 独立纠错不在范围内：那是此前已接受的取舍，不再包装成「完全无损」。

## Answer（提交 7d45454）

**真正的修复在 `doc.rs` 的种子筛选，不在 `symspell.rs`。** 这一点要说清楚，避免下次误读：

- `deletes_seed_terms` 原来只放行 `<= MAX_FUZZY_CHARS (12)` 的 tokens，13–18 字符的真实名称词元
  根本没被喂进删除索引——它们连一个删除变体都没有，一阶纠错自然无从谈起。
  新增 `long_name_tokens`：`12 < n <= symspell::MAX_TERM_CHARS(18)` 且
  `name` / `display` 里真的包含它的词元，接在 `tokens` 之后入种子。
  用 `contains` 卡一道，是为了只认「原文里真有这个词」，不把派生词、拼音串一并放开。
- `symspell.rs` 只是把既有的硬编码上界抽成命名常量：`len > 18` → `MAX_TERM_CHARS`，
  `len > 10` → `FIRST_ORDER_ONLY_CHARS`。**行为逐字节不变**（`git diff` 可核对：只有一处
  文档注释与两处常量替换）。所以「放宽了 SymSpell 上界」这个说法是错的——
  上界本来就是 18，缺的是种子那一侧。
- 长词元的斜率仍由 `symspell::build` 统一判定（>10 只做一阶），没有在 `doc.rs` 里再定一套阈值。

### 测试

- `long_name_token_gets_first_order_deletes_only`（`doc.rs::memory_shape_tests`）：
  `advancedinstaller`（17 字符）的一阶删除变体可反查回原词元，且删除索引键数 <= 20
  （距离 2 会到数百）；`openhardwaremonitor`（19 字符）不进删除索引，内存水位靠这条线守住。
- `long_name_token_one_letter_typo_still_recalls`（`src/search/tests.rs`）：
  用户可见口径 —— `advancedinstalmer` 仍能召回 `Advanced Installer`。
- 短词元路径未改动：`MAX_FUZZY_CHARS` 及距离 2 的集合保持原样。
