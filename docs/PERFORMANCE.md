# Kite 性能分析报告

**测量日期：2026-09-16** · **Windows 11 x64 build 26200** · **Intel i5-13490F** · **32 GB RAM** · **Rust 1.97.1**

本版覆盖当前仓库 release（v0.2.8 + 单例启动等未发布改动）的 100 次连续唤起实测、空闲驻留 CPU/内存，以及搜索引擎 release 基准。数值来自本机当前构建，是基线数据，不代表所有硬件环境。上一轮 2026-09-14 基线见 [`performance-kite-100-20260914.csv`](performance-kite-100-20260914.csv)。

## 1. 测试对象与方法

| 项目 | 本轮 |
|---|---|
| 版本 | 当前仓库 `cargo build --release`（ProductVersion 0.2.8） |
| 索引规模 | 本机扫描 **297** 条应用 |
| 唤起方式 | 全局热键 Alt+Space → Win32 窗口可见 → Esc 隐藏 |
| 循环 | 100 次，间隔 20 ms |
| 内存指标 | `PrivateMemorySize64`、`WorkingSet64` |
| CPU | 启动完成、索引就绪后静默采样 20 秒的 `TotalProcessorTime` 增量 |
| 搜索引擎 | `cargo test --release bench -- --ignored --nocapture`（合成索引 80 / 2000） |

复现：

```powershell
cargo build --release
.\scripts\measure-performance.ps1 -Iterations 100 -IdleSeconds 20
cargo test --release bench -- --ignored --nocapture
```

原始数据：[`performance-kite-100.csv`](performance-kite-100.csv)，汇总 JSON：[`performance-kite-20260916-011716.summary.json`](performance-kite-20260916-011716.summary.json)，曲线：[`performance-100.svg`](performance-100.svg)。

## 2. 100 次连续唤起

| 指标 | 本版 | 2026-09-14 基线 | 变化 |
|---|---:|---:|---:|
| 样本数 | 100 | 100 | — |
| 唤起平均 | **3.91 ms** | 16.60 ms | 更快 |
| 最快 / 最慢 | 3.33 / 32.58 ms | 4.50 / 43.30 ms | — |
| P50 / P95 / P99 | **3.60 / 4.03 / 4.59 ms** | — / 32.52 / 35.96 ms | P95/P99 明显更低 |
| 私有内存平均 | **59.69 MB** | 14.95 MB | **约 4.0×** |
| 私有内存范围 | 59.62–59.72 MB | 13.41–15.81 MB | 稳定但抬升 |
| 工作集平均 | **78.87 MB** | 54.06 MB | 约 1.5× |
| 工作集范围 | 78.84–78.90 MB | 52.58–54.50 MB | 稳定 |

![Kite 100 次连续唤起曲线](performance-100.svg)

说明：

- 首次唤起 32.58 ms，与上轮首样本量级接近；其后稳定在约 3.3–4.6 ms。
- 本轮脚本用 `MainWindowHandle` + `IsWindowVisible` 判定可见，与上轮 Win32 窗口状态读法一致，但环境与窗口激活路径可能有差异；跨轮对比看数量级，不把 3.91 ms 与 16.60 ms 当成严格同口径回归结论。
- 内存在 100 次循环中几乎不增长（59.62→59.62 MB），未见泄漏式爬升。
- 日志侧：空查询排序约数十 µs（见 §4 empty-query），热键到 `show issued` 的应用内路径远小于完整可见耗时。

## 3. 驻留资源与体积

| 指标 | 本版 | 2026-09-14 基线 |
|---|---:|---:|
| 索引就绪后 20 秒静默 CPU 增量 | **0.031 s**（约 0.15% 均核占用） | 0 s |
| 静默私有内存 | 59.72 MB | 14.86 MB |
| 静默工作集 | 78.90 MB | 47.17 MB |
| `kite.exe` | **9.00 MiB**（9,541,632 B） | 7.90 MiB |
| 本机索引条数 | 297 | （上轮未在报告中写明） |

功能堆叠后的驻留成本主要体现在**私有内存**：相对上轮基线约 +45 MB。当前未做分项剖析；候选来源包括更大的检索索引与 launch-group 结果、图标缓存、SQLite/历史、Everything SDK 映射、托盘/热键/单例等常驻线程结构。CPU 空闲仍可忽略，唤起路径未出现随循环恶化。

## 4. 搜索引擎延迟（release bench）

合成索引，单位 µs，QPS = 每秒可完成查询次数。

### 索引 80 条（接近本机规模）

| 查询类型 | query | p50 | p95 | max | QPS |
|---|---|---:|---:|---:|---:|
| exact-en | `chrome` | 82.2 | 132.6 | 373.8 | 11092 |
| prefix-en | `vis` | 64.6 | 96.3 | 2878.4 | 12802 |
| fuzzy-en | `chorme` | 82.6 | 111.5 | 159.8 | 11573 |
| substring-en | `studio` | 88.9 | 148.4 | 227.9 | 10062 |
| exact-zh | `微信` | 14.2 | 21.1 | 1356.2 | 50580 |
| pinyin-full | `weixin` | 89.5 | 142.5 | 200.0 | 9825 |
| pinyin-initial | `wxkf` | 57.0 | 81.0 | 117.2 | 16688 |
| pinyin-initial-short | `wx` | 56.5 | 84.9 | 137.8 | 15837 |
| one-char | `v` | 9.7 | 12.4 | 16.6 | 100220 |
| no-hit | `zzzzqq` | 61.7 | 110.6 | 207.2 | 14610 |
| empty-query | `(打开启动器)` | 38.3 | 66.2 | 124.1 | 22948 |
| index-build | (快照) | 9947 | | | |

### 索引 2000 条（压力档）

| 查询类型 | query | p50 | p95 | max | QPS |
|---|---|---:|---:|---:|---:|
| exact-en | `chrome` | 81.7 | 86.0 | 188.4 | 12048 |
| prefix-en | `vis` | 65.5 | 68.8 | 145.0 | 15042 |
| fuzzy-en | `chorme` | 82.9 | 90.2 | 216.3 | 11772 |
| substring-en | `studio` | 88.5 | 95.4 | 272.0 | 11046 |
| exact-zh | `微信` | 15.8 | 17.2 | 780.7 | 54062 |
| pinyin-full | `weixin` | 89.1 | 97.0 | 158.8 | 11030 |
| pinyin-initial | `wxkf` | 56.9 | 58.3 | 94.7 | 17464 |
| pinyin-initial-short | `wx` | 121.2 | 136.6 | 297.2 | 8000 |
| one-char | `v` | 27.6 | 30.9 | 38.9 | 35596 |
| no-hit | `zzzzqq` | 61.7 | 64.4 | 127.3 | 15994 |
| empty-query | `(打开启动器)` | 94.7 | 98.7 | 173.2 | 10399 |
| index-build | (快照) | 1772979 | | | |

要点：

- 常用查询（exact/prefix/fuzzy/pinyin）在 80 与 2000 档都在约 0.06–0.15 ms 量级，远低于一帧。
- 单字符与中文 exact 更快；`wx` 在 2000 档升到约 0.12 ms，仍可接受。
- 空查询（打开启动器）80 档 p50 约 38 µs，2000 档约 95 µs。
- 索引快照构建：80 条约 10 ms，2000 条约 1.77 s（一次性路径，不在按键热路径上）。

Windows 设置标准词路径（46 入口）额外见 `cargo test --release system_vocabulary_latency`：`文件` 这类短词在 baseline/enriched 下均为数十 µs 级；`activation` 等长英文词约 0.7–0.8 ms。

## 5. 结论

1. **速度**：本机 release 在索引就绪后，Alt+Space 唤起 P50 约 **3.6 ms**、P95 约 **4.0 ms**；搜索引擎按键路径普遍 **&lt;0.2 ms**。功能增加后唤起仍保持在个位数毫秒。
2. **内存**：空闲私有内存约 **60 MB**、工作集约 **79 MB**，相对 09-14 基线明显抬升，是当前最值得后续拆分的驻留成本。100 次唤起无泄漏爬升。
3. **CPU**：静默 20 秒 CPU 增量约 **0.03 s**，可视为无后台空转。
4. **未覆盖**：输入字符到结果列表更新的端到端 UI 延迟仍未自动化；Everything 文件模式、多显示器 DPI 切换、安装器升级路径不在本轮范围。

若要继续压内存，建议先做一次私有字节分项（索引/图标缓存/DB/DLL 映射），再决定是否延迟加载 Everything SDK 或压缩图标缓存驻留。
