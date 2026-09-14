# 03: 扫描深度是否足够（快扫遗漏排查）

**What to build:** 确认「扫描很快但可能不够深」：对照开始菜单/桌面/App Paths 实际目录树，检查 Fast/Full 两档的深度、时间预算、每目录上限是否漏掉可启动应用；在需要时加深或调整预算，并补充回归。

**Blocked by:** None (can start immediately)

**Status:** needs-triage

## Acceptance

- [ ] 列出 Fast/Full 当前 depth / budget / max_per_dir / max_total 与真实开始菜单树的差距
- [ ] 在本机对比「文件系统应有快捷方式」vs「索引条目」的遗漏清单
- [ ] 若有系统性遗漏，调整参数并回归；若无，记录结论关闭

## Comments

- 2026-01: 用户反馈「扫描实测很快，但是深度是不是不够？」——待排查。
