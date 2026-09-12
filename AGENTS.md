# Kite

Windows 11 轻量桌面启动器（对齐 uTools 搜索体验、Flow Launcher 轻量占用）。

产品需求见 `docs/Kite 产品需求文档.md`，UI 参考 `docs/Kite-UI设计图.png`。  
可参考实现：[ZeroLaunch-rs](https://github.com/ghost-him/ZeroLaunch-rs)（仅作参考，不要整仓拷贝）。

## 技术栈（已锁定）

- Tauri 2 + React 19 + TypeScript + Vite + npm
- Rust `stable-x86_64-pc-windows-msvc`（禁止 GNU/MinGW）
- **单 crate**：`src-tauri`，禁止拆 workspace / packages
- 搜索、扫描、启动等核心逻辑必须在 Rust；React 只做 UI 与键盘交互
- 未经阶段需要不要加：SQLite、Tailwind、UI 组件库、状态库、插件系统

## 本机工具链（勿重复安装）

| 组件 | 位置 |
|------|------|
| RUSTUP_HOME / CARGO_HOME | `D:\Tools\rustup` / `D:\Tools\cargo`（系统环境变量） |
| cargo/rustc | `D:\Tools\cargo\bin`（Machine PATH） |
| MSVC Build Tools | `D:\Tools\VS2022BuildTools` |
| Node | `D:\Program Files\nodejs` |
| WebView2 | 已预装 |

## 常用命令

```powershell
npm install
npm run tauri dev      # 开发（Alt+Space 切换窗口）
npm run tauri build    # 发布
cd src-tauri; cargo check; cd ..
```

## 目录规范

```
Kite/                         # 仓库根 = 项目根（禁止 Kite/Kite 嵌套）
├── docs/                     # 需求、UI 图、logo（勿当业务代码改）
├── src/                      # React 仅 UI
│   ├── App.tsx               # 搜索主界面
│   ├── types.ts              # 与 Rust 对齐的 IPC 类型
│   └── App.css               # 对齐 UI 设计图的样式 token
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs            # 入口、AppState、IPC command 注册
│   │   ├── model.rs          # AppItem / SearchResult
│   │   ├── scanner.rs        # 开始菜单/桌面/App Paths 扫描去重
│   │   ├── search.rs         # Exact / Prefix 搜索与排序
│   │   ├── launcher.rs       # 启动进程（禁止拼 shell 字符串）
│   │   ├── icons.rs          # 提取并缓存应用图标
│   │   └── window.rs         # 显示/隐藏/聚焦
│   ├── icons/                # 应用自身 icon（32/128/256/512 + ico）
│   └── tauri.conf.json
└── package.json
```

规则：

- 新 Rust 模块：优先放进上表职责，不要为“可能用到”空建目录
- React 组件若拆分：同功能放 `src/` 扁平文件；超过 2 层再考虑 `src/components/`
- 产物 `dist/`、`src-tauri/target/`、`node_modules/` 不入库

## 代码治理

### 注释

- **写清 Why，不复述 What**。例如「Vite 绑 127.0.0.1：本机默认只听 ::1」
- 模块顶可有一行职责说明；复杂 Windows/`unsafe` 旁注明失败后果
- 禁止大段模板注释、禁止注释掉旧代码长期挂着
- 公开 IPC command 参数/返回值用简短 rustdoc 一行即可

### 命名与结构

- Rust：`snake_case` 函数/模块；导出结构体字段稳定给前端序列化用 `snake_case`（serde 默认）
- 前端：与 Rust 对齐的字段名（`display_name` 等）不要擅自 camelCase 两套
- 评分常量集中在 `search.rs`，不要散落在各个 matcher
- 单个文件职责单一；`lib.rs` 不写业务算法

### 依赖

- 加 crate 前先问：PRD 当前阶段是否需要？能否用标准库？
- Windows API：优先 `windows` crate 最小 feature，不要引入完整 GUI 框架
- 禁止：Electron、React 以外的前端框架、多 crate workspace

### IPC

```
React → invoke(search_apps | launch_app | rescan_apps) → Rust
```

- 不要把完整应用列表推给前端过滤
- 搜索只返回 Top N（当前 10）
- 启动只执行索引里的 target，禁止把用户输入拼进 shell

## 易踩坑

1. **Vite 必须绑 IPv4**：`vite.config.ts` 的 `server.host = "127.0.0.1"`，否则 `tauri dev` 一直等前端。
2. **Windows 下用 `npm.cmd`**，不要对 `npm` 做 `Start-Process`。
3. **esbuild 脚本被拦**：`npm approve-scripts esbuild`。
4. **MSVC 链接失败**：查 Build Tools / `x86_64-pc-windows-msvc`，别装 MinGW。
5. **图标提取失败不能拖垮扫描**：`icons.rs` 失败返回 `None`。
6. **失败先读真实错误**，不要连续猜测改配置。

## 阶段边界（Phase 1）

已完成/目标：`Alt+Space` 唤起、应用扫描去重、图标、Exact/Prefix、↑↓/Enter/Esc、启动后隐藏。

尚未做（勿提前实现）：拼音、Alias、Fuzzy、历史排序、SQLite、托盘、设置页、插件、文件搜索。

搜索回归用例以后修一个 bug 就加一条，见 PRD §63。
