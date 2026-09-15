# 04: 结果降权与恢复

**Status:** ready-for-agent
**Progress:** implemented

- [x] SQLite `demoted` 表 + `demote_item` / `undemote_item` / `demoted_ids`
- [x] `Personalization.demoted`；非保护层扣 `DEMOTE_PENALTY`（120）
- [x] 右键「降低此结果优先级 / 恢复优先级」
- [x] 不删除索引项、不改启动目标；Name Exact 保护层不被挤出
- [x] 存储往返 + 排序主缝测试通过

## Notes

- 与 Pin 可并存；matched_by 可带 `+demote`
- 清空历史不影响 demoted（与 pin 同级）
