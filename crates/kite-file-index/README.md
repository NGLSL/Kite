# Kite 文件索引内核

Kite 内置文件搜索的 Windows Rust 内核。产品不调用 Python、PowerShell 或 experiments 中的程序；服务注册、SCM、ACL、IPC、UI 属于调用方。

## 调用顺序

```rust,ignore
let captured = capture(&new_protected_root, &volumes, cancel.clone(), &progress)?;
// 服务保持只读 metadata watcher，feed 固定在同一受保护 session 下。
watch(&captured.checkpoint, &feed, cancel, &progress)?;
```

`capture` 创建新目录，卷只接受 `C:` 形式的盘符。服务必须事先确保父目录、session 路径和权限属于当前已认证 SID，不能把来自普通 IPC 的任意路径直接交给特权代码。内核只在这个新根下使用固定产物名，仅读卷元数据，不修改 USN、不读取文件内容。layout 仅返回 Win32 50 时才改用经过验证的 ENUM + exact GET_RECORD + 有限 native hardlink 例外路线；其它错误直接失败。完整元数据 capture 可由普通 builder 严格 finalize，不发布部分索引。

普通 controller：

```rust,ignore
let generation = build(&capture_root, &new_generation_root, cancel.clone(), &progress)?;
let mut search = LiveSearch::open_with_cancel(&generation, &feed, cancel)?;
let result = search.query(&query)?;
```

- `build` 仅向新 generation 目录输出；成功才写 `generation.json`。调用方保留旧 generation，等新库真实 Ready 后切换。
- `Generation::load(root)`、`CaptureManifest::load(root)` 读取固定目录布局并校验路径与完整性。
- `Query`：`query: String`、`limit: usize`（0..1000）、`filter: FileFilter`、`excluded_folders: Vec<PathBuf>`。
- 类别：`all/images/documents/videos/audio/archives/folders`，扩展名与 Kite 既有 FileFilter 一致。
- JSON 查询响应：`query/total/paths/paths_utf16/is_dirs/elapsed_ms/live/last_seq`；异常 UTF-16 保留原始数组，不用显示替代字符拼接实际打开路径。
- 排除按 Windows ordinal 忽略大小写、完整目录边界、自身及子树匹配；逐路径过滤后才计数/限量，保留同一文件位于其它目录的硬链接。底层快照仍完整保存，这是搜索结果隐藏，修改立即生效。

## 追平、停止与恢复

服务 watcher 从原始 immutable checkpoint 读取 USN，不能从上次退出游标续接尚未持久化的 overlay。每次 watcher 启动先取得本次全卷 cutoff；全部 cursor 达到 cutoff 且 pending 为零才写 `caught_up` marker。原 begin/generation/provenance 不变。

`LiveSearch::open_with_cancel` 校验 marker 的 generation、scope、全卷 serial/journal/cursor，完成后才释放启动缓存并返回 Ready。仅 feed 已有 begin、或读取到当前文件长度，均不足以 Ready。reader error、停止、heartbeat 超时、identity/retention 改变、overlay 达上限均有明确失败/stale 状态；journal retention 不足返回可 downcast 的 `NeedsRebuild`。正常 controller 应提供重建入口，不持续显示实时状态。

`watch` 无日级或 2100 秒生产上限，持续到共享 `Arc<AtomicBool>` 取消。关闭插件/退出 Kite 时调用方设置取消，停止服务 reader，并 drop `LiveSearch`；Drop 会停止并 join feed reader。索引构建、metadata API 循环和较大 bootstrap 扫描有取消检查。`open_recorded` 仅供完整旧 feed 的离线检查，可接受有序 end，状态仍为 `journal_stopped=true/live=false`，不用于产品实时 Ready。

服务只接受自己受保护 session 中的 checkpoint/feed；普通 AppData 中的 checkpoint 副本是验证材料，不能成为 SYSTEM 任意读取/写入目标。服务捕获目录生命周期由服务 owner 管理。

## 磁盘布局与临时内存

最终 generation 保留 `corpus.bin`（KFSNAP02 原始 UTF-16）、`names.index`（KFPAGED1）、`identities.sorted`、`journal-start.json`、`identities.sorted.provenance.json`、`generation.json`。RAM builder 临时映射快照、共享名字字典并直接写压缩名字块；build 结束释放，搜索继续使用固定 buffer 的 NO_BUFFERING 读取。

成功 build 后清理本次新目标中逐卷 `C/` 等目录下的 `snapshot.bin`、可选 `snapshot.bin.json`、`identities.raw`，以及根下合并的 `identities.raw`；只删固定生成文件并尝试删除空目录，不递归清除。清理失败通过 `cleanup-warning` progress 报告，最终库仍可用。失败构建不发布 generation，保留诊断材料；源服务 capture 不由普通 builder 删除。

现有格式明确要求 snapshot 小于 4 GiB；名字块上限 64 KiB，生成索引采用 4096-byte 对齐 Windows unbuffered I/O，无静默 cached fallback。范围只排除 MFT ordinal0..15 的文件对象，保留所有目录；其它零链接内部对象仍需同 FRN/type/serial 严核，不按名字过滤子树。

## 验证结果

原型迁入内核时，真实基库的 corpus、压缩名字索引和身份索引逐字节一致；录制增量回放、查询顺序、原始 UTF-16、类别与逐路径排除对照通过。集成后已完成真实三卷的重建、重启补齐、实时同步及停止读取验证。

当前集成验证、同一真实日志的优化前后对照、Release 就绪时间与常驻内存集中记录在[文件搜索验证摘要](../../docs/FILE-SEARCH-VALIDATION.md)。各阶段原始资料仅在本机 `.data/` 保留。
