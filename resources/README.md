# resources

- `open.wav`：Kite 窗口唤起音效，由 winmm PlaySound 播放。
- `official-plugins/`：构建生成的官方插件清单和 exe，包含 Loci 只读查询客户端。
- `loci/`：构建生成的 `loci-service.exe`、普通权限 CLI `loci.exe` 与管理员服务安装脚本 `install-service.ps1`。

普通文件模式与显式 `loci` 插件共用 LociIndex 服务的固定 NTFS 卷索引；安装／启用服务需要管理员授权，Kite 日常运行无需提权。原有目录验证引擎的 4096 条目预算不是全盘服务的产品上限。服务范围、构建与尚未验证行为见 [Loci 说明](../docs/LOCI-PLUGIN.md)。安装包不再包含文件搜索 SDK DLL。
