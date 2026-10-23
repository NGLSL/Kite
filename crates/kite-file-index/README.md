# Kite 文件索引内核

Kite 内置文件搜索的 Windows Rust 内核。产品不调用 Python、PowerShell 或 experiments 中的程序；服务注册、SCM、ACL、IPC、UI 属于调用方。

## 调用顺序

```rust,ignore
let captured = capture(&new_protected_root, &volumes, cancel.clone(), &progress)?;
// 服务保持只读 metadata watcher，feed 分段文件固定在受保护 session 下。
watch_resumable(&captured.checkpoint, &feed, cancel, &progress, resume, acknowledged)?;
```

`capture` 创建新目录，卷只接受 `C:` 形式的盘符。服务必须事先确保父目录、session 路径和权限属于当前已认证 SID，不能把来自普通 IPC 的任意路径直接交给特权代码。内核只在这个新根下使用固定产物名，仅读卷元数据，不修改 USN、不读取文件内容。layout 仅返回 Win32 50 时才改用经过验证的 ENUM + exact GET_RECORD + 有限 native hardlink 例外路线；其它错误直接失败。完整元数据 capture 可由普通 builder 严格 finalize，不发布部分索引。

普通 controller：

```rust,ignore
let generation = build(&capture_root, &new_generation_root, cancel.clone(), &progress)?;
// 复用 generation 时先将恢复游标传给服务 watch_path_from。
let resume = LiveSearch::resume_cursors(&generation)?;
let mut search = LiveSearch::open_with_resume_and_ack(
    &generation, &feed, cancel, acknowledge,
)?;
let result = search.query(&query)?;
```

- `build` 仅向新 generation 目录输出；成功才写 `generation.json`。调用方保留旧 generation，等新库真实 Ready 后切换。
- `Generation::load(root)`、`CaptureManifest::load(root)` 读取固定目录布局并校验路径与完整性。
- `Query`：`query: String`、`limit: usize`（0..1000）、`filter: FileFilter`、`excluded_folders: Vec<PathBuf>`。
- 类别：`all/images/documents/videos/audio/archives/folders`，扩展名与 Kite 既有 FileFilter 一致。
- JSON 查询响应：`query/total/paths/paths_utf16/is_dirs/elapsed_ms/live/last_seq`；异常 UTF-16 保留原始数组，不用显示替代字符拼接实际打开路径。
- 排除按 Windows ordinal 忽略大小写、完整目录边界、自身及子树匹配；逐路径过滤后才计数/限量，保留同一文件位于其它目录的硬链接。底层快照仍完整保存，这是搜索结果隐藏，修改立即生效。

## 追平、停止与恢复

服务 watcher 保留原始 immutable checkpoint 作为身份锚点。首次从原始游标读取；复用 generation 时，controller 从 `live-state.sqlite` 读取最近完整提交的全卷 cursors，通过 `watch_path_from` 发送给服务。服务先校验受保护路径与原 provenance，再要求恢复数组与原卷集合、顺序及 journal 身份一致，游标不早于原始起点；内核还会检查当前 journal 连续性。全卷 cutoff 追平后才写 `caught_up` marker。

每轮同步冻结全卷读取终点，轮流读取 64 KiB 页。pending 身份保存在临时 SQLite 中，缓存上限 4 MiB，跨页仍按身份合并，完整一轮统一刷新。输出帧控制在 128 项／约 1 MiB；消费方以完整逻辑轮次事务提交，保留目录变换与当前路径的同轮语义。对象总量增长使用磁盘空间，65,536 个存活对象和 64 MiB 不再是变化状态总容量限制。暂时打不开、权限失败或 delete-pending 仍不等于身份已经消失。

身份回收使用 `identity_only: true` 记录，所有 paths 必须为空；消费方只 shadow 该身份的基库条目并移除同 key 状态，不递归删除目录后代、不改 prefix。真正删除的子项分别由其 USN 身份记录处理，存活或移出的后代保留到最终当前路径刷新。补读和较长的刷新期间持续输出心跳，最终发布前重新校验全卷 journal identity 与 retention。

持久变化对象、shadow、prefix 与全卷恢复 cursors 在 `live-state.sqlite` 的同一事务提交。重启恢复最近完整状态，只重读尚未提交的部分。原基库不可变，查询直接合并基库与磁盘变化表；按身份 upsert／删除、有限缓存、WAL checkpoint 和空闲页复用控制运行开销，暂不全量重写基库。

feed 按约 32 MiB 切段，序号在同一 session 内持续递增。消费端持久消费每帧后发送单向 ACK；服务仅在活动 watch 接收，回收已完整消费的旧段，取消原单会话 512 MiB 寿命上限。服务协议为 3；握手拒绝旧版组合，旧组件需要用户授权更新。现有基库、checkpoint 和索引格式保持可复用。

`LiveSearch::open_with_resume_and_ack` 校验 begin 的恢复身份，以及追平 marker 的 generation、scope、全卷 serial/journal/cursor，完成后才释放启动缓存并返回 Ready。仅 feed 已有 begin、或读取到当前文件长度，均不足以 Ready。reader error、停止、heartbeat 超时、identity/retention 改变、磁盘满及写入失败均有明确失败/stale 状态，保留最后完整提交状态；journal retention 不足返回可 downcast 的 `NeedsRebuild`。正常 controller 应提供重试／重建入口。

`watch` 无日级或 2100 秒生产上限，持续到共享 `Arc<AtomicBool>` 取消。关闭插件/退出 Kite 时调用方设置取消，停止服务 reader，并 drop `LiveSearch`；Drop 会停止并 join feed reader。索引构建、metadata API 循环和较大 bootstrap 扫描有取消检查。`open_recorded` 仅供完整旧 feed 的离线检查，可接受有序 end，状态仍为 `journal_stopped=true/live=false`，不用于产品实时 Ready。

服务只接受自己受保护 session 中的 checkpoint/feed；普通 AppData 中的 checkpoint 副本是验证材料，不能成为 SYSTEM 任意读取/写入目标。服务捕获目录生命周期由服务 owner 管理。

## 磁盘布局与临时内存

最终 generation 保留 `corpus.bin`（KFSNAP02 原始 UTF-16）、`names.index`（KFPAGED1）、`identities.sorted`、`journal-start.json`、`identities.sorted.provenance.json`、`generation.json`。运行时新增 `live-state.sqlite` 及可能存在的 `-wal`／`-shm`，与 generation 同生命周期；成功切换新库且旧 engine 释放后按固定名单清理。RAM builder 临时映射快照、共享名字字典并直接写压缩名字块；build 结束释放，基库搜索继续使用固定 buffer 的 NO_BUFFERING 读取，变化层使用有缓存上限的 SQLite 访问。

成功 build 后清理本次新目标中逐卷 `C/` 等目录下的 `snapshot.bin`、可选 `snapshot.bin.json`、`identities.raw`，以及根下合并的 `identities.raw`；只删固定生成文件并尝试删除空目录，不递归清除。清理失败通过 `cleanup-warning` progress 报告，最终库仍可用。失败构建不发布 generation，保留诊断材料；源服务 capture 不由普通 builder 删除。

现有格式明确要求 snapshot 小于 4 GiB；名字块上限 64 KiB，生成索引采用 4096-byte 对齐 Windows unbuffered I/O，无静默 cached fallback。范围只排除 MFT ordinal0..15 的文件对象，保留所有目录；其它零链接内部对象仍需同 FRN/type/serial 严核，不按名字过滤子树。

## 验证结果

原型迁入内核时，真实基库的 corpus、压缩名字索引和身份索引逐字节一致；录制增量回放、查询顺序、原始 UTF-16、类别与逐路径排除对照通过。历史集成版本已完成真实三卷的重建、重启补齐、实时同步及停止读取验证。

本次磁盘变化层、恢复游标和分段 ACK 尚待真实 USN 大批变化、重启、轮转及崩溃恢复验收。保留的 8,433 项真实日志可用于离线回放；历史结果与性能数据按原阶段保留，不能直接作为本次实现的验收结论。本轮没有新增测试用例，正式打包、安装验证和上传交由 GitHub Actions。

当前集成验证、同一真实日志的优化前后对照、Release 就绪时间与常驻内存集中记录在[文件搜索验证摘要](../../docs/FILE-SEARCH-VALIDATION.md)。各阶段原始资料仅在本机 `.data/` 保留。
