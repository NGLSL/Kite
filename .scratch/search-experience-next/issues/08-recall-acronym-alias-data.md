Status: resolved
Type: task
Blocked by: 06 搜索评估基线

## Goal

支持通用英文多词首字母缩写（如 `ndm` → Neat Download Manager），并以受控内置 Alias 数据维护明确的跨语言同义称呼（如「雷蛇」→ Razer）；用户 Alias 优先，不创建虚假可启动 target。

## Scope

- 通用 Acronym：从英文应用名首字母生成可匹配形式；限制过短片段与误召回。
- 受控内置 Alias 表：常见、明确的中英品牌同义称呼，匹配已索引目标名称片段。
- 用户 Alias 优先于内置 Alias；歧义词不自动点名某一应用。
- 不按品牌写扫描或打分特例；别名不能绕过入口索引。
- 可与票 07 共享召回管线改造，但本票聚焦缩写与别名数据/规则。

## Acceptance Criteria

- [x] 固定样本中 `ndm` 能召回 Neat Download Manager（名称已在索引时）。
- [x] `雷蛇` 能召回已索引的 Razer 应用；无 Razer 索引时不返回虚假项。
- [x] 用户自定义 Alias 与自动缩写冲突时，用户 Alias 仍优先。
- [x] 通用缩写不把无关短名应用全部打进首屏（基线对比）。
- [x] 相对票 06 基线相关样本改善或持平，无明显回退。

## Validation

- `cargo test`（alias / matcher + search_cases）
- 对比基线指标。

## Dependencies

- Blocked by: 06
- 建议在 07 之后合入，避免与 Token/Compact 改动同时大面积调分；若实现互不冲突可并行。

## Out of Scope

- 为任意品牌自动机器翻译别名。
- LLM/向量召回。

## Comments

2026-09-14 实现记录：

- `word_acronym`：≥2 词英文名首字母精确匹配，分 SCORE_ACRONYM=680。
- 受控别名：雷蛇→razer、罗技→logitech、赛睿→steelseries（仅匹配已索引名称片段）。
- 评估样本 ndm / 雷蛇 升为 required；`cargo test` 176 passed。
