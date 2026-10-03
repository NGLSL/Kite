# Loci 文件搜索与后台服务

Kite 的普通文件模式和官方 `com.kite.loci` 插件共用 **LociIndex** Windows 服务维护的索引。服务以 LocalSystem 运行，枚举本机固定 NTFS 卷；Kite、插件及独立查询 CLI 以普通权限通过本机 named pipe 发送只读查询和状态请求。插件不自行扫描目录，不拥有第二套索引；插件进程退出后服务继续运行。

## 使用

安装／启用 LociIndex 服务需要管理员授权；日常运行 Kite 和搜索无需管理员权限。通过管理员安装程序启用服务，服务不可用时结果中显示状态及安装指引。开发者也可在管理员 PowerShell 中调用 Loci 仓库的安装脚本，必须指定服务可执行文件：

```powershell
.\scripts\install-service.ps1 -ExecutablePath D:\Project\Loci\target\release\loci-service.exe
```

上面命令从 Loci 仓库执行。安装脚本注册后台服务并启动；需要验证真实安装、停止、升级与权限行为，不能用编译或协议测试代替。

普通文件模式直接发送查询，无需 `loci` 前缀，文件类型筛选随请求交给服务。显式插件入口支持 `loci 关键词` 和 `loci status`；`loci` 空输入显示服务状态及安装指引。`root`／`save`／`rebuild` 不再是插件管理动作。服务维护保存和恢复，普通客户端不能改变索引范围或触发特权写入。插件查询最多返回 50 条路径，Enter 由 Kite 打开结果；插件请求期限为 500 ms，超时显示服务状态。

独立 `loci.exe` 同样只读查询服务，输出 JSON：

```powershell
loci status
loci query --type documents --limit 50 report
```

支持 `--type all|images|documents|videos|audio|archives|folders` 和 `--limit 1..100`。CLI 的整个请求期限为 6 秒，与插件的 500 ms 请求期限分别约束。

## 索引和结果边界

NTFS 后端通过 MFT 全卷枚举取得对象，再以 `FILE_ID_BOTH_DIR_INFO` 目录批次补齐名称与硬链接关系；在线变化使用 USN 增量及 checkpoint，库存与游标以原子快照保存。排除 NTFS 内部元数据和精确的 `%ProgramData%\Loci` 存储子树，避免保存自身触发索引循环；不会把整个 ProgramData 排除。reparse 条目与访问失败遵守后端范围规则，当前全卷搜索不等同任意文件系统支持。

原有目录 `engine` CLI 的 4096 条目、128 目录及 16 深度预算属于旧验证引擎，**不是全盘服务的产品上限**。服务建立或校正期间的结果有 pending／错误状态；旧结果不代表新完整库存。未配对 UTF-16 名称在索引中保留原始身份，当前 JSON 结果跳过无法安全表示的名称并报告数量，避免把错误转换路径交给打开文件动作。

百万合成库已经实际运行，但这不代表真实百万文件或真实全卷服务已验收。本轮尚未完成管理员服务安装、普通权限端到端查询、真实全卷长期运行及安装升级验收；没有已达到 Everything 性能的测量结论。

## 开发与构建

Kite 与 Loci 仓库放在同一父目录；官方插件通过 `../../../Loci` 的 Cargo path dependency 复用服务协议客户端，服务独立拥有 NTFS 后端。无需复制引擎源码。

```powershell
cargo +1.99.0 test --manifest-path official-plugins/Cargo.toml -p kite-plugin-loci --locked
.\scripts\build-official-plugins.ps1
cargo test stdio_backend_spawns_real_official_plugins --locked -- --nocapture
```

构建输出包含 `resources/official-plugins/com.kite.loci/` 的插件清单／exe，以及 `resources/loci/` 中的 `loci-service.exe`、`loci.exe` 和 `install-service.ps1`。运行安装脚本及管理员授权属于服务安装阶段，构建不会替代安装验收。

CI／release workflow 使用并列仓库和 `scripts/loci-revision.txt` 固定 Loci 提交。当前 pin `476937db24fb8c690d76bd5ab4b9f8c747aadec9` 包含新服务 API，已在本地提交。远端构建需要先推送该 Loci 提交，再验证对应的精确提交 CI；本地构建通过不能替代远端 CI 验证。
