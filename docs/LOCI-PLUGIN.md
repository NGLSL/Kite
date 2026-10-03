# Loci 文件搜索与后台服务

Kite 的普通文件模式和官方 `com.kite.loci` 插件共用 **LociIndex** Windows 服务维护的索引。服务以 LocalSystem 运行，枚举本机固定 NTFS 卷；Kite、插件及独立查询 CLI 以普通权限通过本机 named pipe 发送只读查询和状态请求。插件不自行扫描目录，不拥有第二套索引；插件进程退出后服务继续运行。

## 使用

Loci 独立发布为 Kite 插件包。用户从 [Loci 发布页](https://github.com/NGLSL/Loci/releases) 下载 `loci-kite-plugin.zip`，解压后在 Kite 的插件设置中导入含 `plugin.json` 的目录。Kite 当前导入的是文件夹，不能直接选 ZIP。随后输入 `loci`，点击“安装服务”，接受一次管理员授权，在安装向导中完成安装。安装器会将服务和 CLI 安装到独立的 Program Files\Loci 目录并启动 LociIndex 服务；完成后用 `loci status` 查看状态。

插件包包含查询插件和 `loci-setup.exe`。只有用户点击安装按钮才唤起管理员授权；Kite 和插件日常以普通权限运行。服务与插件进程独立，关闭 Kite、禁用或卸载插件都不会停止或删除 Loci 服务。服务由独立 Loci 卸载入口管理，卸载保留索引数据。

Kite 安装器只包含 Kite 自己的官方插件，不捆绑 Loci 服务或 Loci 插件。没有导入 Loci 时，文件搜索显示导入指引；构建 Kite 不需要 Loci 源码或已安装的服务。插件通过 `\\.\pipe\Loci.Search.v1` 的 v1 协议只读连接后台服务。

独立 Loci 安装后的 CLI 可用绝对路径调用：

```powershell
& "$env:ProgramFiles\Loci\loci.exe" status
& "$env:ProgramFiles\Loci\loci.exe" query --type documents --limit 50 report
```

默认安装到 Program Files\Loci；自定义目录时使用实际路径。服务安装、停止、升级和权限行为仍须实机验收，编译和协议测试不能替代。

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

只需克隆或 fork Kite，Loci 插件源码和打包由 Loci 仓库维护，Kite 通过插件 API 接入，不依赖 Loci 源码。CI 和 release workflow 不 checkout Loci，Kite 安装器不管理独立 Loci 服务。

```powershell
.\scripts\build-official-plugins.ps1
.\scripts\build-installer.ps1
```

Kite 安装器输出为 `artifacts/kite-setup.exe`。Loci 插件包和安装器由 Loci 仓库的 `scripts/build-installer.ps1` 独立生成。源码构建和协议测试无需安装后台服务；真正的文件搜索端到端测试需要单独安装服务。
