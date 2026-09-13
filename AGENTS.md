# Kite 开发指南

这份文件是贡献者和自动化 Agent 的入口摘要。项目细则、目录约定和测试要求以 [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) 为准；业务术语见 [`docs/CONTEXT.md`](docs/CONTEXT.md)，架构决策见 [`docs/adr/`](docs/adr/)。

## 项目定位

Kite 是使用 Tauri 2、React 19、TypeScript 和 Rust 构建的 Windows 轻量桌面启动器。Rust 负责扫描、索引、搜索、排序、启动和系统集成；React 负责输入、结果展示、键盘交互和设置界面。

## 常用命令

```powershell
npm install
npm run tauri:dev
npm run build
npm run tauri:build

cd src-tauri
cargo check
cargo test
cd ..
```

开发环境应使用 Rust stable 的 `x86_64-pc-windows-msvc` 目标，以及带有 Desktop development with C++ 工作负载的 Visual Studio Build Tools。具体安装位置由贡献者自行配置，项目不得依赖某台机器的绝对路径。

## 代码边界

- Rust：应用扫描、去重、搜索、排序、启动、持久化和 Windows 系统集成。
- React：搜索输入、结果列表、键盘交互、设置和状态展示。
- `commands/` 只负责 IPC 适配；不要在命令层复制业务逻辑。
- IPC 只返回排序后的 Top N；不要把完整应用列表交给前端过滤。
- 启动只执行索引中明确记录的 target，禁止把用户输入直接拼进 shell 命令。

## 目录约定

- Rust 业务代码放入 `src-tauri/src/app/`、`search/`、`storage/` 或 `system/` 的对应模块。
- Tauri 命令放入 `src-tauri/src/commands/`。
- 前端功能代码放入 `src/features/`，跨功能复用代码放入 `src/shared/`。
- `src/App.tsx` 和 Rust `lib.rs` 只负责装配，不承载复杂业务逻辑。
- 新代码进入已有职责目录，避免在仓库根目录平铺模块。

建议将 Rust 单文件控制在约 250 行以内、TS/TSX 控制在约 200 行以内；当文件持续增长或职责混杂时，拆分为职责清晰的子模块。重复逻辑第二次出现时再抽取复用，避免没有现实需求的过度抽象。

## 质量与验证

- 搜索行为变更：运行 `cd src-tauri; cargo test`，并为新的搜索问题补回归用例。
- 前端变更：至少运行 `npm run build`。
- 构建成功不等于运行时验收；涉及窗口、快捷键、扫描或启动时，应补充实际 Windows 运行验证。
- 失败先读取真实错误，再做最小修复；不要用猜测式 fallback 掩盖问题。
- 单个应用解析、图标提取或 target 失败不得拖垮整个索引，应跳过或降级并保留必要的调试信息。

## 文档和工具

需求不明确时，先对照 `docs/` 中的 PRD 和 ADR 再实现。模块职责复杂时使用项目提供的 domain modeling 或 codebase design skill；难以复现的故障使用 diagnosing-bugs skill；功能完成后按改动规模进行 code review。不要为了简单改动自动运行全部流程。

Issue 和功能规格放在 `.scratch/<feature-slug>/`，格式约定见 [`docs/agents/issue-tracker.md`](docs/agents/issue-tracker.md)。

## 范围约束

当前项目聚焦 Windows 应用和文件启动体验。插件、AI、OCR、云同步、账户系统、跨平台支持和自建全文索引不属于当前默认范围，除非需求明确变更。
