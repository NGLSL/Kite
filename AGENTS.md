# Kite

Windows 轻量启动器（Tauri 2 + React 19 + Rust MSVC）。  
需求：`docs/Kite 产品需求文档.md` · UI：`docs/Kite-UI设计图.png` · **开发规范：`docs/DEVELOPMENT.md`（目录、文件规模、复用、skills）**

## 命令

```powershell
npm install
npm run tauri dev
npm run tauri build
cd src-tauri; cargo check; cd ..
```

工具链在 D 盘（`RUSTUP_HOME`/`CARGO_HOME`/MSVC），见 DEVELOPMENT.md §7。

## 结构（禁止平铺堆叠）

- Rust：`app/` 扫描启动 · `search/` 搜索 · `system/` 窗口图标 · `commands/` 仅 IPC · `lib.rs` 仅装配
- 前端：`features/search/` UI · `shared/` 复用 · `types/ipc.ts` 与 Rust 对齐 · `App.tsx` 仅组合
- 文件规模、注释、禁止重复实现 → **`docs/DEVELOPMENT.md` §2–4**

## Skills（后续任务必须遵守）

技能在 **`.agents/skills/`**，场景表见 **`docs/DEVELOPMENT.md` §5**。

- 含糊需求先 `grilling` / 对照 PRD 用 `grill-with-docs`
- 模块边界用 `domain-modeling` / `codebase-design`；难 Bug 用 `diagnosing-bugs`
- 功能合入前视改动用 `code-review`；跨会话大需求先议清再 `to-spec`
- **不要默认全跑**；简单小改直接实现


## 硬约束

- 搜索逻辑只在 Rust；IPC 只回 Top N
- 启动只执行索引 target，禁止拼用户输入进 shell
- Phase 1–3 完成；Phase 4 与 Everything 已接入：托盘、开机启动、失焦隐藏、设置、用户 Alias、UWP、文件搜索  
  **仍不做**：插件、AI、OCR、云同步、跨平台
- 搜索回归：`cd src-tauri; cargo test`
- 领域词汇：`docs/CONTEXT.md` · ADR：`docs/adr/`
- `vite.config.ts` 保持 `host: 127.0.0.1`；失败先读真实错误
## Agent skills

### Issue tracker

Issues and specs live as Markdown files under `.scratch/<feature-slug>/`.
See `docs/agents/issue-tracker.md`.

### Domain docs

This is a single-context repo. Read `docs/CONTEXT.md` and relevant files under `docs/adr/`.
See `docs/agents/domain.md`.

### Triage labels

Use the canonical Triage states in `Status:` lines. See `docs/agents/triage-labels.md`.
