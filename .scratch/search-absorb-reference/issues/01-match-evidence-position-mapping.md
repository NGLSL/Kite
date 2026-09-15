# 01: 匹配证据位置映射回原名称

**What to build:** 拼音、紧凑文本等派生字段上的命中，在可映射时映射回原名称字符位置（真实 start/span）；不可映射时明确记为未知，不得写成 0 或最优位置。统一最终排序的同分比较继续使用证据细排：更早起点、更少跳空、更长连续覆盖稳定靠前。无个性化与有个性化路径共用同一规则。用户侧体感是「同样能匹配，更自然的命中排在前面」。

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent
**Progress:** implemented；待正式 `/code-review` 与提交

- [x] 派生字段（拼音/紧凑等）命中可映射时，MatchEvidence 记录原名称上的真实起点与跨度
- [x] 不可映射位置记为未知语义，禁止伪装成起点 0 / 最优
- [x] 有序多词/跳字证据继续携带真实间隔，不统一写 gaps=0
- [x] 主缝测试：同分双候选中，可映射为更早/更紧凑原名称位置者靠前；未知位置不得战胜真实前缀位置
- [x] 明确匹配保护、质量层与既有偏好样本不回退
- [x] `cargo test` 全绿；相对既有 required eval 无显著回退

## Notes

- token-seq / skip / ib-pinyin / mixed-pinyin 写入真实 start。
- skip 不得挡住更高分 fuzzy（修复 `crome`→Google Chrome）。
- 顺带拆分 verify 职责：`evidence` / `context` / `align` / 编排 `mod`。
