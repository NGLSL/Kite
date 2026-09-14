Status: resolved
Type: task
Blocked by: 06 搜索评估基线

## Goal

在现有 Normalizer / Matcher / Ranker 分工内加入 Token Match、Compact Name 与 Word Prefix，使 `todo`、`visual code`/`vs code`、`ter` 等分词、连写与单词前缀输入可靠召回，且不按应用名新增算法分支。

## Scope

- Normalize/Tokenize 产出原名、紧凑名称（去词间空白）与单词边界。
- Matcher 区分 full exact、compact exact、word exact/prefix、ordered token 等；分数常量集中在 Ranker。
- 样本锁定：`Microsoft To Do` 的 `todo`；`Visual Studio Code` 的 `visual code`/`vs code`；`XTerminal` 的 `ter`。
- 自动缩写/紧凑匹配低于真实名称精确匹配与用户 Alias；限制短片段误召回。
- 原始名称、原始 Query、Alias 与启动 target 保持不变。

## Acceptance Criteria

- [x] `todo` 能把 `To Do`/`Microsoft To Do` 召回进 Top1 或约定 Top3，而不再只剩网页搜索。
- [x] `visual code` 与 `vs code` 能稳定命中 Visual Studio Code。
- [x] `ter` 在无 History 时能把 XTerminal 放进前排候选（具体位次以样本期望为准）。
- [x] 短 Query 不因紧凑匹配大量挤入无关系统工具。
- [x] 相对票 06 基线：相关样本 Top1/MRR 改善或持平，无样本显著回退。
- [x] 不为某个产品名写 `if name == ...` 分支。

## Validation

- `cargo test`（search matcher/ranker + search_cases）
- 对比票 06 基线指标。

## Dependencies

- Blocked by: 06

## Out of Scope

- 多词首字母通用缩写与跨语言品牌别名（票 08）。
- History 分层（票 09）。

## Comments

2026-09-14 实现记录：

- Normalizer：`compact` / `tokens` / `split_camel`。
- Matcher：compact exact/substring（最短 3 字）、word exact/prefix（Camel 拆词）、有序多词（含连续词首字母 `vs`→visual+studio）。
- Ranker 分数：compact-exact 920、token-seq 880、word-exact 850、compact-sub 780、word-prefix 720。
- 评估：todo / visual code / vs code / ter 均 Top1，已从 known-gap 升为 required。
- `cargo test` 175 passed。
