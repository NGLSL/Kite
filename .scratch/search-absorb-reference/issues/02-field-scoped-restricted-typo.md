# 02: 拼音/别名字段受限纠错

**Status:** ready-for-agent
**Progress:** implemented

- [x] 删除索引候选可追溯；验证按字段做真实距离（pinyin / initials / keyword 全拼）
- [x] 不用查询侧删除深度 == 0 丢候选
- [x] `jisunqi` 召回「计算器」；`jisuanqi` 不回退
- [x] 长度/距离上限保留；产品名中立
- [x] deletes 预算测试与全量 `cargo test` 通过

## Notes

- 验证通道标签：`pinyin-fuzzy`
- deletes 种子保持 perf 预算，不回灌 compact/context
