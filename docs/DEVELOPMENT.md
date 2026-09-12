# Kite 开发规范

Agent 与人类开发都按本文执行。`AGENTS.md` 只保留入口摘要，细则以本文为准。

## 1. 职责边界

| 层 | 负责 | 不负责 |
|----|------|--------|
| Rust | 扫描、去重、搜索、排序、启动、系统集成 | UI 布局、样式 |
| React | 输入、结果列表、键盘、空/错状态 | 搜索算法、索引 |

IPC：`React invoke → commands.rs → app/search/system`。禁止把全量应用列表丢给前端过滤。

## 2. 目录与文件规模

**禁止平铺堆叠。** 新代码进已有职责目录；根上只保留装配文件。

领域词汇：`docs/CONTEXT.md`（勿放仓库根）。ADR：`docs/adr/`。

```
src-tauri/src/
  lib.rs / main.rs / model.rs / state.rs / commands.rs
  app/launcher.rs
  app/scanner/{mod,lnk,registry,util}.rs
  search/{mod,matcher}.rs
  history.rs
  storage/            # SQLite 历史库
  system/window.rs
  system/icons/{mod,extract}.rs

src/
  App.tsx / main.tsx
  types/ipc.ts
  shared/          # 跨功能复用
  styles/global.css
  features/search/ # SearchPanel + 子组件 + useSearch + search.css
```

| 类型 | 软上限 | 超限 |
|------|--------|------|
| Rust 单文件 | ~250 行 | 拆子模块 |
| TS/TSX 单文件 | ~200 行 | 拆组件/hook |
| CSS | ~200 行 | token → global，组件样式就近 |
| `lib.rs` / `App.tsx` | ≤80 行 | 只装配 |

命名：Rust `snake_case`；IPC 字段与 serde 一致（`display_name`），前端不要另起一套 camelCase。

## 3. 复用（禁止重复实现）

- 同一逻辑出现 **第 2 次** → 抽到 `src/shared/` 或 Rust 对应 `util` / 子模块
- UI 形态复用 → 抽组件（例：`ResultItem`），父组件只传 props
- 跨 feature 共用 → `src/shared/`，禁止两处拷贝
- **不要过度抽象**：只出现 1 次的先不抽；命名必须说明职责
- 评分常量集中在 `search/mod.rs`，禁止散落 matcher

## 4. 注释与依赖

- 注释一律用 **中文**，写 **Why**，不复述 What；`unsafe` 旁写清失败后果
- 加 crate / npm 依赖前：当前阶段是否真需要？能否用标准库？
- 禁止：Electron、非 React 前端、workspace 多 crate、为插件提前拆 crate

## 5. 项目 Skills 用法

技能在 `.agents/skills/`（见 `skills-lock.json`）。**按场景用，不要每个任务全跑一遍。**

| 时机 | Skill |
|------|--------|
| 需求含糊、先追问再写 | `grilling` |
| 对照 PRD/UI 图等已有文档拆任务 | `grill-with-docs` |
| 模块职责/边界要理清 | `domain-modeling` / `codebase-design` |
| 难复现 Bug | `diagnosing-bugs` |
| 跨会话大需求 | `to-spec`（先议清再套） |
| 功能合入前 | `code-review`（按改动大小，可抽查） |
| 查外部实现/资料 | `research` |

简单小改直接实现；不要为“规范”自动拉全量 skill。

## 6. 阶段与质量

- 只做当前 Phase 需要的；用户 Alias 持久化 / 历史 / SQLite / 托盘 / 插件属后续
- 搜索回归：`cd src-tauri; cargo test`；修 bug 往 `search/mod.rs` 的 `assert_top` 加用例
- Phase 2：拼音索引时预计算；分数集中在 `search/ranker.rs`；同分短名优先
- 失败先读真实错误再最小修复

## 7. 本机环境（勿重复安装）

| 项 | 值 |
|----|-----|
| RUSTUP_HOME / CARGO_HOME | `D:\Tools\rustup` / `D:\Tools\cargo` |
| MSVC | `D:\Tools\VS2022BuildTools` |
| Node | `D:\Program Files\nodejs` |
| Toolchain | `stable-x86_64-pc-windows-msvc` |

```powershell
npm install
npm run tauri dev
npm run tauri build
cd src-tauri; cargo check; cd ..
```

## 8. 易踩坑

1. `vite.config.ts` 保持 `server.host = "127.0.0.1"`
2. Windows 脚本用 `npm.cmd`；VS Code 若找不到 `cargo`，用 `npm run tauri:dev` 或整窗重启
3. esbuild 被拦：`npm approve-scripts esbuild`
4. 链接失败查 MSVC，不装 MinGW
5. 图标提取失败返回 `None`，不得拖垮扫描
6. **旧 `kite.exe` 未退出时再 `tauri dev` 会报 `HotKey already registered`** — 先结束进程再启动
