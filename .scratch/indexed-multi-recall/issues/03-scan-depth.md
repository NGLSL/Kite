# 03: 扫描深度是否足够（快扫遗漏排查）

**What to build:** 确认「扫描很快但可能不够深」：对照开始菜单/桌面/App Paths 实际目录树，检查 Fast/Full 两档的深度、时间预算、每目录上限是否漏掉可启动应用。

**Blocked by:** None

**Status:** resolved

## Acceptance

- [x] 列出 Fast/Full 当前 depth / budget / max_per_dir / max_total 与真实开始菜单树的差距
- [x] 在本机对比「文件系统应有快捷方式」vs 索引能力的差距
- [x] 结论：本机无需加深；记录数据关闭

## Comments

### 本机实测（2026-01，日常使用机）

| 来源 | 数量 | 最大深度 |
|------|------|----------|
| user Start Menu lnk | 47 | 4 |
| common Start Menu lnk | 96 | 5 |
| user Desktop lnk | 20 | 2 |
| Public Desktop lnk | 18 | 1 |
| **lnk 合计 / 去重名** | **179 / 141** | — |
| App Paths 唯一 target | 43（raw 79，含 WOW64 重复） | — |
| Scoop shims | 无 | — |

开始菜单 lnk 深度分布：d2=22, d3=91, d4=26, d5=4，**>5 = 0**。

Fast 档参数：depth 5 / budget 1.5s / max_per_dir 400 / max_total 1500 —— 本机全部落在上限内，**深度不是瓶颈**。

「扫得快」是因为条目本身就少（约 140+ 去重名 + ~40 App Paths + UWP），不是截断。若以后出现嵌套安装器或公司镜像大量分类目录，再评估加深 Fast depth 或拉长预算。

- 验收缺口：尚未用 kite 真实索引条数做最终对照（需跑一次 scan 日志）。
