# ADR 0002：文件搜索走 Everything，不自建索引

## 状态

已接受

## 背景

PRD §53：不要自行实现 NTFS 全盘索引；优先 Everything IPC/SDK。

## 决策

- 本机已安装 Everything（`D:\Program Files\Everything`）
- 通过 Everything 命令行导出 CSV（CREATE_NO_WINDOW、短超时）做兜底查询
- 文件结果 score 固定低于应用精确匹配，追加在应用列表后
- Everything 未安装/超时：静默跳过，不影响应用搜索

## 备选

- 完整 Everything.dll SDK / 自定义 IPC 窗口：更实时，但复杂度高，后续可换
- Windows Search：结果与排序不符合启动器习惯

## 后果

- 首次文件命中可能有数百毫秒延迟
- 依赖 Everything 进程与索引状态
