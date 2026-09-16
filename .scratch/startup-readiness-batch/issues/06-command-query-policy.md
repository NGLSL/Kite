Status: ready-for-agent
Type: task
Parent: startup-readiness-batch/spec.md

# 06: 非空 Query 命令可见性与归并

**What to build:** 精确/高质量 Query 仍可召回并启动命令入口（如精确 `7z`）；短 Query、松散模糊匹配不得让 `7z`/`7zfm`/`7zg` 一窝蜂占首屏。已有正式 AppItem 时，对应命令名优先作为召回 Alias 归并，而不是再占独立行。用户 Pin/Alias/明确匹配保护不被降噪压制。

**Blocked by:** 05 来源分层 + 空 Query 隐藏命令裸行

**Status:** ready-for-agent

- [ ] 精确匹配命令入口仍可展示并启动
- [ ] 短/模糊 Query 对命令 Alias 层降可见或归并到正式 AppItem
- [ ] 同启动语义/同名同 stem 时命令名服务正式入口召回；不能确认时不错误合并
- [ ] Pin、用户 Alias、明确匹配保护优先于命令降噪
- [ ] Demote 语义保持：不删索引、不改 target
- [ ] `cargo test` 用稳定 fixture 锁定 7z 族与 `winget`/`python`/`node` 类样本（只写规则，不写产品分支）
- [ ] 实机：模糊搜 7-Zip 不再被 shims 刷屏；精确 `7z` 仍可用
