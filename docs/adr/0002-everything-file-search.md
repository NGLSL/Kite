# ADR 0002：文件搜索走 Everything，不自建索引

## 状态

已接受（2026-09-13 修订：CLI 导出 CSV → SDK DLL IPC；补充：文件模式下文件命中优先于应用）

## 背景

PRD §53：不要自行实现 NTFS 全盘索引；优先 Everything IPC/SDK。

初版用 `Everything.exe -s ... -export-csv` 导出查询：Everything 未在后台运行时，
该命令会**直接拉起 Everything 主窗口**，违背启动器「不喧宾夺主」的预期。

用户在文件模式下期望「找文件时文件排前面」；若仍按应用 MatchScore 排序，短 Query 下应用会淹没文件命中。

## 决策

- **捆绑官方 SDK DLL**（`resources/Everything64.dll`，随安装包发布），
  通过 `LoadLibraryW` + IPC 查询运行中的 Everything 实例；结果在列表中直接显示
- Everything 已安装但未运行时，在结果中提示用户先启动 Everything
- 未检测到 Everything 安装时，在结果中显示可点击的官方下载安装入口
- Kite 绝不自动启动 Everything 窗口
- DLL 查找顺序：exe 目录 → 资源目录 → Everything 安装目录 → 系统 PATH
- **非文件模式**：文件结果 score 固定低于应用精确匹配，**追加在应用列表后**；
  图标按扩展名走系统关联类型图标（`cache_type_icon`）
- **文件模式**（用户显式切换「文件」胶囊时）：
  - Everything 文件命中**优先于**应用等其余命中（模式级优先，不参与 FinalScore 数值竞争）
  - 有文件结果时**不插入**网页搜索槽位，避免打散文件列表
  - 依赖状态行仍置顶；应用命中保留在文件之后，不移除
- Everything 未安装/未运行：显示一条依赖状态结果，不影响应用搜索

## 备选

- 命令行导出 CSV：已废弃，会拉起 GUI 且每次查询 spawn 进程
- 手写 WM_COPYDATA 窗口消息：无 DLL 依赖但要自行维护协议，SDK DLL 仅 91KB
- Windows Search：结果与排序不符合启动器习惯
- 文件模式仍按分数混排：会退回「应用刷屏、文件沉底」的体验，与用户显式意图不符

## 后果

- 查询毫秒级返回（原 CSV 方案为数百毫秒）
- 依赖 Everything 进程与索引状态（与之前一致）
- 安装包多捆绑一个 91KB 的 DLL（SDK 许可允许再分发）
- 文件模式存在模式级排序路径：改 Core Ranker 时不必改文件置顶逻辑，但改「文件模式组装」时须同步 `ui/results` 测试
- 非文件模式仍遵循 ADR 原排序（文件追加在应用后）
