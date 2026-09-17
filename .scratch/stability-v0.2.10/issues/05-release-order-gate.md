# 05: 发布顺序闸门与文档

**What to build:** 维护者发布流程固定为「push main → CI 绿 → 再打 tag → Release」；可选在 Release 增加「tag commit 对应 CI success」校验。不新建第二套 CI。

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] `docs/DEVELOPMENT.md` 正式发布：固定「main 合并 → CI 绿 → 再 tag」
- [x] `docs/releases/README.md` 同步；明确 NSIS/Release 只在 tag，`ci.yml` 是 main/PR 唯一门禁
- [x] `release.yml` 新增「Verify tag commit CI succeeded」：查 `ci.yml` 对该 SHA 的 success run，无则失败并提示先 push main
- [x] 人工走读 workflow YAML；API 失败路径在下次真实发版时验证

## Comments

- 不新建第二套 CI。
- 校验用 `GITHUB_TOKEN` 调 Actions API，查 `head_sha=tag commit` 且 `status=success` 的 ci.yml run。
