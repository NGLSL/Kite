# Kite 开发规范

## 当前架构

Kite 是 Windows x64 原生桌面应用，使用 Rust 2021、Iced 0.14 和 tiny-skia。项目不使用 Tauri、WebView2、React 或 Node.js。

程序入口为 `src/main.rs`，主 crate 负责装配业务和 UI，文件索引内核与按需元数据服务位于 `crates/`。Iced 负责窗口、输入、设置页、托盘交互和渲染；Rust 模块负责扫描、搜索、启动、持久化及 Windows 集成。

## 目录职责

- `src/app/`：应用扫描、快捷方式解析、UWP 和启动逻辑
- `src/search/`：规范化、别名、拼音、召回和排序
- `src/file_search/`：内置文件搜索控制器、索引构建与后台生命周期
- `crates/`：低内存文件索引内核与按需元数据服务
- `src/storage/`：SQLite 设置、固定项和历史
- `src/system/`：快捷键、托盘、图标、音效和 Windows API
- `src/ui/`：Iced 状态、视图、后台任务和交互
- `src/plugin/`：插件 Registry / Activation / Host / 协议（见 `docs/PLUGIN-DEVELOPMENT.md`）
- `official-plugins/`：官方插件 Rust workspace（性能考虑；协议语言无关，不编入主 crate）
- `icons/`：程序及托盘图标
- `resources/`：音效、官方插件打包目录（`official-plugins/`，构建生成）等运行资源
- `installer/`：NSIS 安装脚本
- `scripts/`：构建辅助脚本
- `docs/`：需求、性能、架构决策、插件开发说明和开发规范

## 修改规则

- 先阅读相关模块和 `docs/` 文档，再修改代码。
- `main.rs` 和 `lib.rs` 只负责装配；业务逻辑放入对应职责目录。
- 启动程序只能使用索引中已验证的 target，不得把用户输入拼进 shell 命令。
- 单个快捷方式、图标或 target 失败不得中断整个索引。
- 搜索行为变更必须增加 Rust 回归测试。
- 涉及窗口、热键、托盘、扫描或启动的修改，除编译外要做 Windows 运行验证。
- 保持改动聚焦，避免无需求的抽象、依赖和跨平台扩展。

## 本机必要检查

```powershell
cargo check --locked --offline
git diff --check
```

按改动范围补充已修改文件的格式检查、PowerShell/YAML 解析等静态检查；仅为定位具体故障运行相关既有测试。依赖未缓存时先获取依赖，不把离线条件不足当作源码失败。完整测试、正式 Release 编译、官方插件打包、NSIS、安装验收和上传统一在 GitHub Actions 执行。

需要运行主程序定位界面问题时使用 `cargo run --locked`。本机不为常规提交或发布重复制作安装包，也不覆盖现用 Kite 或读取组件。不要提交 `target/`、`artifacts/` 或本机日志。

## 文件索引诊断

`kite.exe --kite-file-index-session <data-directory>` 提供重定向 stdin/stdout 的逐行 JSON 会话，复用生产文件搜索控制器。它只使用明确给出的数据目录，不自动安装组件或创建窗口；运行前应已安装相同版本的读取组件。Actions 安装验收使用安装后的同一份程序调用此入口。

请求示例：

```json
{"command":"status"}
{"command":"query","query":"文件名关键词","limit":20,"filter":"all"}
{"command":"stop"}
```

每个请求返回一行带 `ok` 的 JSON。查询仅在 `state.phase=ready` 时接受；状态包含 generation、提交游标和活跃 feed 位置。显式 `rebuild` 请求会重新建库；`stop` 或输入 EOF 等待控制器退出并释放服务会话。此入口供诊断和验收使用，查询结果不会被执行。

## 正式发布

发布顺序、工作流分工、安装验收范围及完成标准统一维护在 [`RELEASING.md`](RELEASING.md)。维护者说明格式见 [`releases/README.md`](releases/README.md)。正式发布需要明确授权；仅修改代码或发布流程不会触发推送、打 tag 或上传 Release。
