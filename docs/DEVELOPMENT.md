# Kite 开发规范

## 当前架构

Kite 是 Windows x64 原生桌面应用，使用 Rust 2021、Iced 0.14 和 tiny-skia。项目不使用 Tauri、WebView2、React 或 Node.js。

程序入口为 `src/main.rs`，业务和 UI 位于同一个 Cargo crate。Iced 负责窗口、输入、设置页、托盘交互和渲染；Rust 模块负责扫描、搜索、启动、持久化及 Windows 集成。

## 目录职责

- `src/app/`：应用扫描、快捷方式解析、UWP 和启动逻辑
- `src/search/`：规范化、别名、拼音、召回和排序
- `src/storage/`：SQLite 设置、固定项和历史
- `src/system/`：快捷键、托盘、图标、音效和 Windows API
- `src/ui/`：Iced 状态、视图、后台任务和交互
- `icons/`：程序及托盘图标
- `resources/`：Everything DLL、音效等运行资源
- `installer/`：NSIS 安装脚本
- `scripts/`：构建辅助脚本
- `docs/`：需求、性能、架构决策和开发说明

## 修改规则

- 先阅读相关模块和 `docs/` 文档，再修改代码。
- `main.rs` 和 `lib.rs` 只负责装配；业务逻辑放入对应职责目录。
- 启动程序只能使用索引中已验证的 target，不得把用户输入拼进 shell 命令。
- 单个快捷方式、图标或 target 失败不得中断整个索引。
- 搜索行为变更必须增加 Rust 回归测试。
- 涉及窗口、热键、托盘、扫描或启动的修改，除编译外要做 Windows 运行验证。
- 保持改动聚焦，避免无需求的抽象、依赖和跨平台扩展。

## 验证与打包

```powershell
cargo check
cargo test
cargo build --release
.\scripts\build-installer.ps1
```

安装器脚本会自动构建 release 版本并生成：

```text
artifacts/kite.exe
artifacts/kite-setup.exe
```

需要 NSIS 时执行 `winget install NSIS.NSIS`。不要提交 `target/`、`artifacts/` 或本机日志。

## 正式发布

正式版本只由 `main` 上的 `v*` tag 触发。**禁止**在未合入 main 或 main 上 CI 未绿时打 tag。

固定顺序：

1. 合并目标提交到 `main`，等待 `.github/workflows/ci.yml`（push main / PR）成功。
2. 按 [`docs/releases/README.md`](releases/README.md) 写好 `docs/releases/vX.Y.Z.md`，并更新 `Cargo.toml` 版本。
3. 在 **已绿的 main 提交** 上创建并推送 `vX.Y.Z` tag。
4. Release workflow 会校验：tag 提交包含于 origin/main，且该提交的 CI workflow 成功；然后跑 test、NSIS、生成报告并发布。

NSIS 与完整 Release **只在 tag 上跑**，不在每个 PR 上跑。`ci.yml` 是 main/PR 的唯一自动化门禁。

发布报告会记录 tag 提交、提交范围、`cargo test`、release 构建、NSIS 构建、构建环境、安装包大小和 SHA-256。发布完成后应检查报告内容和下载文件，再向用户公布版本。
