# Kite

Kite 是一个面向 Windows 的轻量桌面启动器。按下快捷键，输入应用名称、别名或拼音，即可快速找到并启动程序。

Kite 使用 Rust 负责扫描、索引、搜索和 Windows 系统集成，使用 Tauri 2 承载桌面窗口，使用 React 和 TypeScript 构建界面。项目关注三件事：搜索结果足够准确、响应足够快、后台资源占用足够低。

> 当前版本面向 Windows x64。项目仍在持续迭代中，欢迎提交 Issue 和 Pull Request。

## 功能

- 全局快捷键唤起和隐藏搜索窗口
- 扫描开始菜单、桌面、注册表 App Paths 以及 Windows packaged apps
- 识别 `.lnk`、`.exe` 和 UWP 应用，并按稳定 target 去重
- 中文、英文、别名、拼音和拼音首字母搜索
- Exact、Prefix、Substring、Fuzzy 多路召回和统一排序
- 用户 Alias、固定结果、启动历史和个性化排序
- 应用图标提取与失败降级
- 键盘操作：上下选择、Enter 启动、Esc 隐藏
- 系统托盘、开机启动、失焦隐藏和手动重新扫描
- 可选的 Everything 文件搜索集成

搜索逻辑全部在 Rust 中执行，前端通过 Tauri IPC 只接收排序后的 Top N 结果。启动应用时只执行索引中记录的 target，不把用户输入拼接进 shell 命令。

## 截图

界面设计稿位于 [`docs/Kite-UI设计图.png`](docs/Kite-UI设计图.png)。

## 技术栈

- [Tauri 2](https://v2.tauri.app/)
- Rust 2021
- React 19
- TypeScript
- Vite
- SQLite（`rusqlite`，用于设置、固定项和使用历史）

## 开始开发

### 环境要求

- Windows x64
- [Node.js](https://nodejs.org/)（建议使用当前 LTS）
- npm
- [Rust](https://www.rust-lang.org/tools/install) stable 工具链
- Visual Studio Build Tools 的 **Desktop development with C++** 工作负载
- WebView2 Runtime（Windows 10/11 通常已预装）

Rust 应使用 MSVC 目标：

```text
stable-x86_64-pc-windows-msvc
```

### 安装依赖并运行

```powershell
npm install
npm run tauri:dev
```

如果只需要启动 Vite 前端：

```powershell
npm run dev
```

### 构建和检查

```powershell
# TypeScript 类型检查并构建前端
npm run build

# 构建 Windows 安装包（MSI / NSIS）
npm run tauri:build

# Rust 编译检查和搜索回归测试
cd src-tauri
cargo check
cargo test
cd ..
```

首次构建会下载 Rust 和 npm 依赖，所需时间取决于网络和本地缓存。不要把本机的 `RUSTUP_HOME`、`CARGO_HOME`、Visual Studio 或 Node.js 安装路径写入项目配置；请通过系统工具链和环境变量管理它们。

### 使用 GitHub Actions 构建

仓库内置了 `.github/workflows/build.yml`。在 GitHub 仓库的 **Actions → Build Windows installers → Run workflow** 中手动触发后，Actions 会在 `windows-latest` Runner 上构建 MSI 和 NSIS 安装包，并将它们作为 `kite-windows-installers` Artifact 提供下载。推送 `v*` 标签则由 Release workflow 创建 GitHub Release。
## 项目结构

```text
src/                         React 界面和前端共享模块
  features/search/           搜索窗口、结果列表、设置
  shared/                    跨功能复用逻辑
  types/ipc.ts               与 Rust 对齐的 IPC 类型
src-tauri/src/               Rust 应用核心
  app/                       应用扫描、解析和启动
  search/                    规范化、召回、拼音和排序
  commands/                  Tauri IPC 命令
  storage/                   设置、固定项和历史数据
  system/                    快捷键、窗口、托盘和 Windows 集成
docs/                        PRD、领域词汇、ADR 和开发规范
```

更完整的模块边界、文件规模和测试约定见 [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)。业务术语见 [`docs/CONTEXT.md`](docs/CONTEXT.md)。

## 开发约定

- 搜索和启动逻辑放在 Rust，React 只负责输入、展示和交互。
- IPC 返回排序后的有限结果，不把完整索引发送到前端。
- 单个快捷方式损坏、图标读取失败或 target 不存在时，应跳过或降级，不能让整个索引失败。
- 新增搜索行为时，同时补充 `src-tauri` 中的回归测试。
- 优先复用现有模块，避免为尚未实现的插件、AI、OCR、云同步或跨平台能力提前增加抽象。

## 当前范围

Kite 当前专注于 Windows 应用和文件启动体验。插件市场、AI 助手、OCR、云同步、账户系统、Linux/macOS 支持和自建全文索引不在当前范围内。

## 参与贡献

欢迎通过 Issue 报告问题、提出功能建议，或提交 Pull Request。提交代码前请：

1. 阅读 [`AGENTS.md`](AGENTS.md) 和 [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)。
2. 说明变更的用户行为和验证方式。
3. 至少运行与改动相关的前端构建或 Rust 测试。

## 许可证

本项目使用 [Apache License 2.0](LICENSE) 发布。