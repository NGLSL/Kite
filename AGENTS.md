# AGENTS.md

## 项目

Kite 是面向 Windows 的轻量应用启动器，使用 Rust 2021、Iced 0.14 和 tiny-skia 构建原生桌面界面。项目不使用 Tauri、WebView2、React 或 Node.js。

## 目录

- `src/app/`：应用扫描、解析和启动
- `src/search/`：规范化、召回、排序和拼音
- `src/file_search/`：内置文件搜索控制器与后台生命周期
- `crates/`：低内存文件索引内核与按需元数据服务
- `src/storage/`：SQLite 设置、固定项和历史
- `src/system/`：Windows 快捷键、托盘、图标和系统集成
- `src/ui/`：Iced 窗口、设置页和交互
- `resources/`：音效、官方插件打包目录（`official-plugins/`，构建生成）等运行资源
- `official-plugins/`：官方插件 Rust workspace（性能考虑；协议语言无关，见 `docs/PLUGIN-DEVELOPMENT.md`）
- `icons/`：应用图标
- `installer/`：NSIS 安装脚本
- `scripts/`：构建辅助脚本
- `docs/`：需求、架构决策、插件开发说明（`PLUGIN-DEVELOPMENT.md`）和开发文档

## 开发规则

- 先阅读相关模块和 `docs/` 文档，再修改代码。
- 业务逻辑放在职责模块中；`src/main.rs` 和 `src/lib.rs` 只负责装配。
- 启动程序只能使用索引中已验证的 target，不得把用户输入拼进 shell 命令。
- 单个快捷方式、图标或 target 失败不得拖垮整个扫描流程。
- Windows 行为变更要考虑真实运行时验证，不把编译通过当作完整验收。
- 修改搜索行为时补充 Rust 回归测试。
- 保持改动聚焦，避免无需求的抽象、依赖和跨平台扩展。

## 本机检查与发布

本机按改动范围执行必要的编译、格式和静态检查；仅在定位具体问题时运行相关既有测试。常规提交和发布不在本机重复全量测试、Release 构建、NSIS 打包、安装或覆盖升级。

```powershell
cargo check --locked --offline
git diff --check
```

正式构建、安装验收、校验和与产物上传统一由 GitHub Actions 执行。发布版本或修改发布流程时先读 [`docs/RELEASING.md`](docs/RELEASING.md)，以此处的项目流程为准；完整 CI 通过后才在对应提交打注释 tag。安装验证失败阻止发布。没有单独授权时，不安装、替换或重启本机现用 Kite 和读取组件。
