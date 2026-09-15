# ADR 0002：文件搜索走 Everything，不自建索引

## 状态

已接受（2026-09-13 修订：CLI 导出 CSV → SDK DLL IPC）

## 背景

PRD §53：不要自行实现 NTFS 全盘索引；优先 Everything IPC/SDK。

初版用 `Everything.exe -s ... -export-csv` 导出查询：Everything 未在后台运行时，
该命令会**直接拉起 Everything 主窗口**，违背启动器「不喧宾夺主」的预期。

## 决策

- **捆绑官方 SDK DLL**（`resources/Everything64.dll`，随安装包发布），
  通过 `LoadLibraryW` + IPC 查询运行中的 Everything 实例；结果在列表中直接显示
- Everything 已安装但未运行时，在结果中提示用户先启动 Everything
- 未检测到 Everything 安装时，在结果中显示可点击的官方下载安装入口
- Kite 绝不自动启动 Everything 窗口
- DLL 查找顺序：exe 目录 → 资源目录 → Everything 安装目录 → 系统 PATH
- 文件结果 score 固定低于应用精确匹配，追加在应用列表后；
  图标按扩展名走系统关联类型图标（`cache_type_icon`）
- Everything 未安装/未运行：显示一条依赖状态结果，不影响应用搜索

## 备选

- 命令行导出 CSV：已废弃，会拉起 GUI 且每次查询 spawn 进程
- 手写 WM_COPYDATA 窗口消息：无 DLL 依赖但要自行维护协议，SDK DLL 仅 91KB
- Windows Search：结果与排序不符合启动器习惯

## 后果

- 查询毫秒级返回（原 CSV 方案为数百毫秒）
- 依赖 Everything 进程与索引状态（与之前一致）
- 安装包多捆绑一个 91KB 的 DLL（SDK 许可允许再分发）
