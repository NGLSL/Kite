Status: ready-for-agent
Type: task
Blocked by: None

## Goal

在改评分或召回之前建立可复现评估基线：固定 `tests/search_cases.json` 记录 Query、稳定目标身份与 Top1/Top5 期望；可运行地统计 Top1 Accuracy、Recall@5、MRR、重复入口数与输入到列表更新延迟。

## Scope

- 新增固定 Query/索引样本集（中英文、别名、拼音、系统工具、原始 exe、友好入口）。
- 每个样本记录：`query`、固定索引 fixture、`expected_top`、`expected_in_top5`、`forbidden_top`、适用历史状态。
- 期望指向稳定 fixture 身份，不只依赖可能重名的显示名。
- 索引覆盖率单独以 Windows 可启动入口为分母统计，避免把未入索引误判为 matcher 失败。
- 先跑改动前基线并记录，再逐项加特征比较；不因单个截图调分数。
- 产品名只作为样本数据，不在扫描或排序规则中写特例。

## Acceptance Criteria

- [ ] `tests/search_cases.json`（或等价固定样本）存在且可被测试/脚本加载。
- [ ] 评估程序输出 Top1 Accuracy、Recall@5、MRR、重复入口数、P50/P95 延迟（至少在样本集上）。
- [ ] 样本覆盖规格点名场景：`To Do`/`todo`、`visual code`/`vs code`、`vsc`、`wt`、`ndm`、`雷蛇`/Razer、`ter`/XTerminal、用户 Alias 与自动首字母冲突、精确 vs 紧凑竞争、短 Query 误召回。
- [ ] 基线结果写入可追踪位置（测试输出或 `.scratch/search-experience-next/` 下报告），后续票可对比。
- [ ] 现有 `search` 模块测试仍通过。

## Validation

- `cargo test`
- 运行评估命令/测试，确认数字可复现（同 fixture 同分）。

## Dependencies

无。为票 07、08、09 提供改前基线与改后对比。

## Out of Scope

- 实际召回特征实现（票 07/08）。
- History 权重重调（票 09）。
- 与 Flow Launcher 的正式对比评测（可作备注，不阻塞本票）。

## Comments
