# 03: Manifest Registry + Command 静态入口

**What to build:** Kite 启动后扫描默认插件目录中的 `plugin.json`，完成校验与注册。启用插件的 Command（标题/关键词）可被搜索到；用户搜普通应用（如 `chrome`）时行为与无插件一致，且**不**因为安装或搜索 Command 而启动任何插件进程。非法 Manifest（路径逃逸、不兼容 Plugin API、重复 Plugin ID）被拒绝或标记为不可用。

**Blocked by:** 01 — Prefactor Expand — ResultSource + ResultAction

**Status:** resolved

- [x] 读取插件目录下的 Manifest，解析 id/name/version/compatibility/runtime/contributes
- [x] 校验：schema_version、plugin_api 兼容、Plugin ID 唯一与字符集、command/icon/资源路径必须落在插件根内
- [x] idle_timeout 请求值 clamp 到 15–300 秒量级；禁止「0 = 永不关闭」语义
- [x] Command 注册进 Core Index/搜索可见结果：可按 title/keywords 命中，结果 source 体现插件来源
- [x] Command 未被选择前不得 spawn 插件可执行文件
- [x] 普通 Query 路径：即使已安装多个插件也保持与无插件基本一致（0 插件进程）
- [x] 路由/注册层单测覆盖：合法 Manifest、路径逃逸、不兼容 API、重复 ID、idle clamp
- [x] `cargo test` 全绿

## Comments

- 非法 Manifest 以 Incompatible + enabled=false 进 Registry，管理页可见。
- `command_hits` 不 spawn；`ordinary_query_never_spawns` 覆盖。

## Notes

- 对应规格 Stage 1 + Command Contribution 的静态部分。
- 测试主缝：路由缝（Registry 纯逻辑）；无需真实进程。
- 数据目录隔离（插件可写 plugin-data，不可写 Kite SQLite/Index）在类型/约定上落实，完整运行时验证放到 Host 票。
