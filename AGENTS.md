# AGENTS.md

## 项目

Kite 是面向 Windows 的轻量应用启动器，使用 Rust 2021、Iced 0.14 和 tiny-skia 构建原生桌面界面。项目不使用 Tauri、WebView2、React 或 Node.js。

## 目录

- `src/app/`：应用扫描、解析和启动
- `src/search/`：规范化、召回、排序和拼音
- `src/storage/`：SQLite 设置、固定项和历史
- `src/system/`：Windows 快捷键、托盘、图标和系统集成
- `src/ui/`：Iced 窗口、设置页和交互
- `resources/`：Everything DLL、音效、官方插件打包目录（`official-plugins/`，构建生成）等运行资源
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

## 验证命令

```powershell
cargo check
cargo test
cargo build --release
.\scripts\build-installer.ps1
```

`build-installer.ps1` 会自动构建 release 版本并生成 `artifacts/kite-setup.exe`。需要 NSIS 时安装 `NSIS.NSIS`。