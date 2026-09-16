# 08: compact 派生字段命中位置映射回原文

**Status:** resolved
**Blocked by:** None (can start immediately)
**Type:** bugfix

**What to build:** 由紧凑／派生字段产生的命中，其位置能对应回用户实际看到的原始名称文本；给出真实的字符下标。对应不上时，明确记为「位置未知」，而不是随便给一个看起来最优的位置。

- [ ] compact-exact 与 compact-substring 命中不再直接把紧凑文本坐标当作原名称起点
- [ ] 可映射时，记录原始展示文本中的真实字符下标（含空白在内的原字符位置，而不是去掉空白后的序号）
- [ ] 不可映射时记为该字段的「未知」语义，不得写成 0，也不得写成最优起点
- [ ] 未知位置在同分细排中不得战胜真实前缀位置
- [ ] 映射函数的单元测试：在含空白的展示名上，验证首字母／紧凑类命中返回的原始字符下标

## Notes

- 拼音侧的位置映射已经完成（首字母串字节偏移 → 原始展示文本字符下标，含空白），本票只补 compact 一类，不重复实现拼音侧。
- 影响面主要是证据细排与评分诊断，不是所有搜索都会出错——**不要借此引入第二个 matcher**。
- 工单语义对齐：派生字段命中可映射时记原名称真实起点、不可映射记未知。此前只完成了拼音那一半。

## Answer（提交 7d45454）

- `align.rs` 新增 `map_compact_index_to_source(source, compact_index)`：compact 是原文去掉空白，
  所以「原文第 n 个非空白字符」就是 compact 第 n 个字符；映射不了返回 `usize::MAX`（未知），
  与 `map_initial_index_to_display` 同一约定。
- `evidence.rs` 新增 `MatchEvidence::exact_at(field, start)`；`exact(field)` 改为
  `exact_at(field, 0)`，让 compact-exact 也能带上真实起点。
- 六个 compact 命中点全部接上映射：compact-exact／compact-substring（name、display）、
  keyword-compact-exact／substring、context-compact-exact／substring。没有残留的裸紧凑下标。

### 测试

- `map_compact_index_to_source` 单元测试（含空白的展示名、越界记未知）——本轮补上跨度断言。
- `compact_position_maps_back_to_source_including_whitespace`：`"to do list"` / `"dolist"`
  起点 == 3、跨度 == 7（不是紧凑文本的 6）。**已做变异验证**：把映射改回返回紧凑长度即失败。
- `compact_hit_position_maps_back_to_display_text`（`diagnose.rs`）：诊断侧起点 == 3，已做变异验证。
- `unknown_position_never_outranks_a_real_one`：未知位置在同分细排中输给真实位置。

## Comments

### 代码复核采纳项

- **start/span 坐标系混用（已修）**：只修了 `start` 时，`span` 仍是紧凑长度——一份证据里混了
  两套坐标系。补 `map_compact_span_to_source`，三个 compact-substring 点统一走
  `compact_contiguous`，起点与跨度都按原文算（跨度含夹在中间的空白）。
  注意 context-compact-substring 原先传的是 `q_len`，现在也改成真实原文跨度。
  `outranks` 只在 start/gaps/edit_cost 全平后才比 span，所以顺序影响可忽略，但坐标系终于统一了。
- **`context_fields` / `context_compacts` 错位隐患（已修）**：原实现里两个数组各自去重
  （字段按原文去重、紧凑串按紧凑串去重），两个不同字段有同一紧凑串时会错位，
  `.zip()` 之后可能把紧凑命中映射到**另一个字段**的原文上。现在两个数组共用同一个
  `!contains` 判定一起入列，严格同序同长。
  副作用：纯空白字段会贡献一个空紧凑串，它永远匹配不上非空 Query，为保持同长而保留（已注释）。

### 测试

- `map_compact_index_to_source` 单元测试（含空白的展示名、越界记未知）——本轮补上跨度断言。
- `compact_position_maps_back_to_source_including_whitespace`：`"to do list"` / `"dolist"`
  起点 == 3、跨度 == 7（不是紧凑文本的 6）。**已做变异验证**：把映射改回返回紧凑长度即失败。
- `compact_hit_position_maps_back_to_display_text`（`diagnose.rs`）：诊断侧起点 == 3，已做变异验证。
- `unknown_position_never_outranks_a_real_one`：未知位置在同分细排中输给真实位置。
