# 03: 收紧混输高分并区分有序对齐与宽召回

**What to build:** 用户做汉字+拼音/英文混输时，按名称顺序完成的对齐（如「微信xin」）仍可高分；仅片段分别碰巧出现、顺序不一致或位置复用的弱关联，不得与有序混拼音同权，只作更靠后的宽召回。影响同分胜负的占位证据（有序多词 gaps、词元字段/起点、不可映射拼音起点）改为真实值或明确「未知」，禁止把未知位置写成最佳位置。

**Blocked by:** 01 贯穿匹配证据的最终排序

**Status:** done

- [x] 整词级高分要求：汉字段与拉丁段按目标字段顺序对齐，或由音节级混合拼音匹配器认可；否则不得使用与有序对齐相同的高分
- [x] 宽召回分档严格低于有序对齐分档；最终同分时证据仍可区分强弱
- [x] 混输验证不重复使用同一名称位置同时满足多个片段
- [x] 占位证据修正：有序多词记录真实间隔；词元命中尽量记录真实字段与起点；拼音不可映射位置记未知而非 0
- [x] 端到端样本：顺序对齐混输进入前排；顺序不一致的片段碰巧命中不得与之同权（允许更后出现）
- [x] 无产品名专属打分分支
- [x] `cargo test` 全绿

## Comments

2026-03-12 实现：

- `ParsedQuery.ordered_mixed_parts` 保留输入顺序
- `ordered_mixed_align`：名称游标 + 拼音游标；拉丁匹配拼音会推进 name 下标，避免 `xin微信` 倒序复用
- 有序 → `mixed-cjk-latin-ordered` + SCORE_WORD_EXACT；宽召回 → `mixed-cjk-latin-loose` + 低分、未知 start 用 MAX
- `ordered_token_gaps` 写入真实 gaps；ib-pinyin 未知 start 记 MAX
- 测试：`ordered_mixed_accepts/rejects`、`mixed_ordered_alignment_scores_higher_than_loose_fragments`、`token_seq_prefers_tighter_name`
