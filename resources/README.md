# resources

- `open.wav`：Kite 窗口唤起音效，由 winmm PlaySound 播放。
- `official-plugins/`：构建生成的官方插件清单和 exe。

普通文件模式与显式 `loci` 插件共用 LociIndex 服务的固定 NTFS 卷索引；Loci 插件包独立发布，导入后点击安装服务，Kite 安装包不捆绑 Loci；安装／启用服务需要管理员授权，Kite 日常运行无需提权。原有目录验证引擎的 4096 条目预算不是全盘服务的产品上限。服务范围、构建与尚未验证行为见 [Loci 说明](../docs/LOCI-PLUGIN.md)。安装包不再包含文件搜索 SDK DLL。
