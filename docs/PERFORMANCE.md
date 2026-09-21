# Kite 性能分析报告

**0.3.7 代码级基线** · **测量日期：2026-09-27** · **Windows 11 教育版 10.0.26200** · **Intel i5-13490F** · **32 GB RAM** · **rustc 1.97.1** · **crate version 0.3.6（0.3.7 基线批次）**

本版默认性能基线**不主动唤起 Kite 窗口**，改为通过代码路径压测热路径、故障恢复与升级兼容。真 UI 唤起延迟（Alt+Space）降级为发布抽检，沿用 `scripts/measure-performance.ps1`；其历史数据见文末附录，**不与本轮代码基线混比**。

> 口径：所有延迟单位为 **µs**（除非注明 ms）；来自本机 `cargo test --release`，是基线数据，不代表所有硬件环境。

## 1. 方法与复现

| 项目 | 本轮 |
|---|---|
| 入口 | `cargo test --release report_perf_baseline -- --ignored --nocapture` |
| 是否启动 UI | **否**（不创建 kite.exe 窗口） |
| 搜索语料 | 合成 AppItem 80 / 800 |
| 插件 | `PluginHost` + `MemProcessBackend`（生产同款 Host 接口） |
| Everything / 恢复 | `EverythingClient` + `ScriptedEverything` + `Recovery::on_system_pulse` |
| DPI / 多屏 | `window_place::center_physical` / `physical_to_logical_for_window` 纯计算 |
| 存储升级 | 旧 `user_version=1` fixture → 当前 `HistoryDb::open`；快照 Warm/Cold；原子写 |
| 原始输出 | [`performance-code-baseline-037.txt`](performance-code-baseline-037.txt) |

复现：

```powershell
cargo test --release report_perf_baseline -- --ignored --nocapture
# 可选：真 UI 唤起抽检
# .\scripts\measure-performance.ps1 -Iterations 100 -IdleSeconds 20 -SettleSeconds 15
```

职责模块：`src/perf/{report,search,plugin,system_pulse,dpi,storage}.rs`；生产 seam：`src/system/everything.rs`（`EverythingClient`）、`src/system/recovery.rs`（`SystemPulse` / `RecoveryEffects`）。

## 2. 搜索 / 索引（反复查询代理）

| 索引条数 | 查询 | p50 | p95 | max | 备注 |
|---:|---|---:|---:|---:|---|
| 80 | `chrome` | 112 | 129 | 176 | |
| 80 | `vis` | 99 | 112 | 184 | |
| 80 | `微信` | 40 | 72 | 126 | |
| 80 | `weixin` | 126 | 155 | 296 | 拼音 |
| 80 | `zzzz` | 75 | 101 | 159 | 无命中 |
| 80 | `v` | 4 | 33 | 35 | 单字符 |
| 80 | empty-query | 18 | 19 | 39 | index-build **4.3 ms** |
| 800 | `chrome` | 59 | 61 | 110 | |
| 800 | `vis` | 48 | 49 | 68 | |
| 800 | `微信` | 6 | 6 | 12 | |
| 800 | `weixin` | 63 | 87 | 176 | |
| 800 | `zzzz` | 39 | 41 | 98 | |
| 800 | `v` | 7 | 7 | 70 | |
| 800 | empty-query | 24 | 25 | 69 | index-build **265.6 ms** |

要点：

- 查询热路径普遍在 **数十～一百多 µs**，远低于一帧。
- 空 Query（打开启动器排序代理）p50 约 18–24 µs。
- 索引快照构建不在按键热路径：80 条约 4.3 ms，800 条约 266 ms。

## 3. 插件崩溃 / 超时

| 场景 | p50 | p95 | max | 不变量 |
|---|---:|---:|---:|---|
| query 复用（懒启动后） | 12 | 17 | 57 | `spawn=1`，`state=Ready` |
| 崩溃后探测 | 19 | 19 | 19 | `Faulted` 后 retry 可恢复（非 crash-loop） |
| query hang | 802079 | — | — | **有界**：约 **802 ms** 返回错误，不永久阻塞 |
| crash-loop 抑制 | — | — | — | 连续 3 次崩溃后 `HostError::CrashLoop`，不再自动 spawn |

要点：故障路径正确且超时上界约等于 `QUERY_HARD_TIMEOUT_MS`（800 ms 量级）。

## 4. Everything 降级 / 系统脉冲恢复

| 场景 | p50 | p95 | max | 不变量 |
|---|---:|---:|---:|---|
| Everything Ready 搜索（脚本命中） | 1 | 1 | 1 | 注入客户端，无真实 IPC |
| Everything 停机后搜索 | 0 | 0 | 0 | 立即空结果 |
| 脉冲：Everything 状态翻转 | 0 | 0 | 3 | 探测后 `InstalledButNotRunning`，动作含清文件结果 |
| ResumeFromSleep / ExplorerRestarted / DisplayTopology / HotkeyLost | 0 | 0 | 0 | `actions` 非空，符合恢复契约 |

要点：恢复接口开销可忽略；Everything 消失时文件搜索路径不阻塞。**真进程重启**仍属 OS-only 抽检。

## 5. DPI / 多屏几何

| 场景 | p50 | p95 | max | 不变量 |
|---|---:|---:|---:|---|
| 4 档显示器矩阵居中 + 逻辑坐标换算 | 0 | 0 | 0 | 结果有限（非 NaN）；覆盖 100% / 150% / 副屏 / 200% |

纯计算路径无窗口副作用；真拓扑热插拔标 OS-only。

## 6. 存储升级 / 原子写

| 场景 | p50 | p95 | max | 不变量 |
|---|---:|---:|---:|---|
| 历史库 `user_version=1` → 当前 open/migrate | 5513 | — | — | **usage_kept=true**；设置可加载 |
| 部分 settings 键加载 | 0 | 0 | 0 | 缺字段回落默认（theme=dark） |
| 快照 Warm load（40 条） | 9497 | — | — | `apps=40 compatible=true` |
| 不兼容 snapshot | — | — | — | **rejected=true**（冷启动，不误用旧 schema） |
| 原子覆盖写 64 KiB | 5760 | 6836 | 7531 | 字节完整 |

要点：升级与兼容路径在代码层可重复验证；不依赖实机覆盖安装。

## 7. 结论（0.3.7 代码基线）

1. **默认基线无需唤起 UI**：一条 `report_perf_baseline` 覆盖搜索、插件故障、Everything/恢复、DPI、存储升级。
2. **热路径**：合成索引下查询 p50 多在 6–126 µs；空 Query 排序约 20 µs 量级。
3. **故障路径**：插件 hang 有界（~800 ms）；crash-loop 会抑制自动重启；Everything 停机搜索立即空返回。
4. **升级路径**：v1 历史库迁移保留 usage；不兼容 snapshot 正确拒绝 Warm。
5. **未纳入本报告**：真 UI 唤起延迟/驻留内存/CPU、真睡眠·Explorer·Everything 进程、多屏热插拔、实机安装器升级。发布前建议抽检 UI 脚本；其余 OS 行为见下方清单。

### OS-only 清单

| 场景 | 方式 |
|---|---|
| 真 UI 唤起延迟 / 空闲内存 | `scripts/measure-performance.ps1` |
| 真实睡眠/唤醒、Explorer 重启、Everything 进程重启 | 实机（代码已覆盖动作集与降级契约） |
| 多屏热插拔 / 系统 DPI 变更 | 实机（几何矩阵已代码覆盖） |
| 覆盖安装 / 旧版升级实机 | 安装器 + 实机；静态契约 `scripts/test-installer-upgrade.ps1` |

## 附录：历史 UI 唤起基线（0.2.8 批次，口径不同）

**测量日期：2026-09-16** · 唤起方式 Alt+Space → 窗口可见 → Esc，100 次。数值**不可**与上文 µs 级代码基线直接对比。

| 指标 | 17:11 |
|---|---:|
| 唤起 P50 / P95 | 2.93 / 3.64 ms |
| 私有内存平均 | 28.32 MB |
| 静默 20 秒 CPU 增量（先等 15 s 收尾） | 0.094 s |
| 索引条数 | 299 |

搜索引擎旧 bench（合成 80/2000、查询类型更全）见命令：`cargo test --release bench -- --ignored --nocapture`。产品 PRD 中的「稳定空闲私有内存约 14.86 MB」来自更早构建；功能叠加后以 0.2.8 批次约 28 MB 为更新参考（口径见该批次原文）。
