# Kite 驻留内存分项分析

**日期：2026-09-16** · 机器与 `docs/PERFORMANCE.md` 同机

## 0. 优化后结果（SymSpell 种子收紧）

| 指标 | 优化前 | 优化后 |
|---|---:|---:|
| `RetrievalIndex::build` heap 增量 | +44.5 MB | **+10.7 MB** |
| deletes 键数 | 139,257 | **21,470** |
| 进程空闲 private | ~58–60 MB | **~28 MB** |
| 进程空闲 working set | ~79 MB | **~57 MB** |
| 搜索回归 | — | 117 通过 |

改动：`deletes_seed_terms()` 仅把名称/显示名/token/短关键词/短拼音送入 SymSpell；compact、系统词 context、超长拼音不再生成距离 2 删除。词长 >10 只做距离 1，>18 不建 deletes。

反馈回路：

```powershell
cargo test --release memory_shape -- --nocapture
cargo test --release report_heap_deltas -- --ignored --nocapture --test-threads=1
.\scripts\measure-performance.ps1 -Iterations 30 -IdleSeconds 10
```

---

## 1. 结论（优化前）

| 分项 | 约估 | 证据 |
|---|---:|---|
| **RetrievalIndex（首次完整构建，含系统入口）** | **~44 MB** | 测试进程 private 增量 |
| └ 其中 **SymSpell `DeleteIndex`** | **主导** | 139k 删除变体键 |
| fontdb `load_system_fonts` 元数据 | ~0.3 MB | 557 faces |
| 检索字符串池 | ~0.28 MB | 字节粗估 |
| 磁盘图标缓存 | 1.1 MB | 512 PNG |

**一句话**：约 60 MB 私有内存里，大头是检索索引的 SymSpell 删除表，不是 UI 帧缓冲。

## 2. 探针方法

```powershell
# 结构规模 + 首次构建 heap 增量（务必单线程，避免并行测试互相污染）
cargo test --release memory_probe -- --ignored --nocapture --test-threads=1
```

进程侧：

```powershell
# VirtualQuery 分桶（CommitPrivate / Image / Mapped）
# 见 .scratch/memory-vq.cs
```

运行中 kite 抽样（索引就绪后）：

- `Get-Process`：private ≈ 58 MB，working ≈ 78 MB，threads ≈ 30+
- VirtualQuery：**CommitPrivate ≈ 55 MB**，其中 **Private/RW ≈ 53.6 MB**（堆）
- CommitImage ≈ 78 MB（共享代码，不算私有膨胀）
- Mapped/RO ≈ 48 MB（文件映射/字体等，多为共享）
- **Everything64.dll 未加载**（文件模式前）

## 3. 检索索引结构（根因）

本机规模：apps 合成 297 + 系统入口 98（其中 winsettings 46，`search_context` 合计 647 词 / ~9 KB 文本）。

首次 `RetrievalIndex::build`（干净堆）private 增量：

```text
+44.5 MB（docs=395）
```

结构计数（`structure_stats()`）：

| 结构 | keys | posting/value 条数 |
|---|---:|---:|
| term_postings | 4,460 | 6,913 |
| compact_postings | 1,766 | 2,081 |
| gram2 | 1,807 | 22,761 |
| gram3 | 5,551 | 24,107 |
| **deletes（SymSpell）** | **139,257** | **162,010** |
| char_postings | 452 | — |

解读：

- 倒排词元本身只有数千量级，gram 也只有一两万条目，合计粗算 **&lt; 2 MB**。
- **`deletes` 键数是 term 的约 31 倍**：`symspell::build(&term_set, 2)` 对 `term_set` 里每个词做最多距离 2 的删除变体；`term_set` 含名称、token、关键词、拼音全拼/首字母、compact、系统词 context 等。
- 每个删除变体是独立 `String` + `HashMap` 槽 + `Vec<String>` 值，allocator 放大后轻松吃掉数十 MB。粗算下界已约 14 MB；与实测 +44 MB 同向。

字符串池粗估（仅 `str.len()`，不含容器）：

- AppItem apps+system ≈ 98 KB
- IndexedDoc 全字段 ≈ 275 KB
- 清空 `search_context` 后 ≈ 213 KB（差约 62 KB 派生）

→ **砍系统标准词原文对驻留几乎无感；砍 SymSpell 生成集合才有感。**

## 4. 字体与 UI

- `fontdb::load_system_fonts`：557 faces，测试进程 private **+0.3 MB**（只扫元数据）。
- 真实进程里 Iced/cosmic-text 会在首绘时加载 Noto Sans SC（磁盘约 17 MB）等，可能映射或部分驻留；相对索引仍属次要。
- 窗口 640×420 的 tiny-skia 面缓冲通常为数 MB 量级，不是 60 MB 的主因。

## 5. 优化建议（按 ROI）

1. **收紧 SymSpell 输入集（首选）**  
   仅对「名称 / 显示名 / 可信关键词」生成 deletes；不要对 compact、拼音全拼串、系统词 context 全量生成。预期可砍掉大部分 139k 键。

2. **限制 deletes 深度与长度**  
   - `max_deletes=1` 或仅对短词（如 ≤8 字符）做距离 2  
   - 跳过超长拼音串与纯 CJK 单字（召回已有 gram/char 通道）

3. **更紧凑的 deletes 表示**  
   键用 `Box<str>` / 驻留 intern，值用 `u32` 词元 id 而不是 `Vec<String>` 原词副本。

4. **不要为了省内存去砍系统词**  
   词文本本身只有 ~9 KB；若要减，应减「进入 deletes 的派生词」，不是删功能。

5. **次要**  
   图标按需解码、Everything 懒加载对私有内存帮助有限（当前未加载 DLL、图标磁盘仅 1.1 MB）。

## 6. 建议验收

优化后在同一机器复跑：

```powershell
.\scripts\measure-performance.ps1 -Iterations 100 -IdleSeconds 20
cargo test --release memory_probe -- --ignored --nocapture --test-threads=1
```

目标量级（非承诺）：空闲私有内存从 ~60 MB 降到 **~25–35 MB**，且搜索 bench 与 100 次唤起无明显回退。

## 7. 代码落点

| 位置 | 说明 |
|---|---|
| `src/search/retrieval/doc.rs` | `symspell::build(&term_set, 2)`；`term_set` 组装；`structure_stats()` |
| `src/search/retrieval/symspell.rs` | 删除变体生成 |
| `src/search/memory_probe.rs` | 分项探针（`#[ignore]` 测试） |
| `.scratch/memory-vq.cs` | VirtualQuery 分桶脚本（本机分析用） |
