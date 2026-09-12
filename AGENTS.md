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

- Rust：`app/` 扫描启动 · `search/` 搜索 · `system/` 窗口图标 · `commands.rs` 仅 IPC · `lib.rs` 仅装配
- 前端：`features/search/` UI · `shared/` 复用 · `types/ipc.ts` 与 Rust 对齐 · `App.tsx` 仅组合
- 文件规模、注释、禁止重复实现 → **`docs/DEVELOPMENT.md` §2–4**

## Skills

在 `.agents/skills/`。按 DEVELOPMENT.md §5 的场景选用，不要默认全跑。

## 硬约束

- 搜索逻辑只在 Rust；IPC 只回 Top N
- 启动只执行索引 target，禁止拼用户输入进 shell
- Phase 1 完成；Phase 2 搜索核心已含：Normalizer / 内置 Alias / 拼音 / Substring / Fuzzy / 统一评分  
  仍不做：用户 Alias 持久化、历史排序、SQLite、托盘、插件
- 搜索回归：`cd src-tauri; cargo test`；修 bug 往 `search/mod.rs` 的 `assert_top` 加用例
- `vite.config.ts` 保持 `host: 127.0.0.1`；失败先读真实错误
