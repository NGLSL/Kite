Status: ready-for-agent
Type: enhancement

# Kite Plugin System V1

目标版本：Kite v0.3.x  
Plugin API：1  
Manifest Schema：1

## Problem Statement

Kite 目前是轻量 Windows Launcher，但「应用搜索」之外的能力若继续塞进 Core，会把启动器越做越重：更多常驻线程、更多索引污染、更多启动路径风险。用户并不希望删除这些能力，而是希望它们**按需出现**——用计算器时就出现计算结果，不需要时完全不影响启动器。

现状的具体痛点：

1. **所有能力与 Core 绑死。** 计算器、翻译、窗口切换、开发者工具等如果做成内置功能，任何一个崩溃、卡顿、内存泄漏都可能拖垮整个启动器；用户无法只卸载自己不用的能力。
2. **搜索热路径会被「能力竞速」污染。** 若多个动态能力都能看到全部 Query，就会与 AppItem 搜索争抢评分与列表，导致用户输入 `chrome` 时行为漂移。
3. **结果模型仍是 AppItem 导向。** 当前 `SearchResult` 直接嵌套 `AppItem`，文件、网页、系统状态、未来插件结果都挤在同一条展示/启动模型上，行为与展示边界不清。
4. **用户无法组装自己的能力。** 第三方开发者没有稳定的本地扩展点；社区能力进不了 Kite，只能另起进程或改 Core。

用户最终感受到的矛盾是：**功能越多，启动器越不像启动器。**

## Solution

引入 **Kite Plugin System V1**，一句话定义：

> 插件负责计算、查询和执行能力，Kite 负责触发、生命周期、结果展示和主要交互。

对用户的体验是：

```text
能力可插拔，体验尽量留在 Kite 内。
```

设计原则（V1 强制）：

```text
能力外置
体验内联
显式激活
按需启动
进程隔离
宿主渲染
失败隔离
```

用户路径：

```text
普通 Query
  → 走 Core Search（AppItem / 既有结果源），插件默认不运行、不参与

显式 Trigger 命中（prefix / keyword）
  → 进入 Provider Mode
  → 懒启动对应插件子进程
  → 插件返回 List / Panel 声明式数据
  → Kite 用 Iced 原生渲染
  → Enter 执行 Native Action 或 Plugin Action
```

验收场景（用户视角）：

- 安装多个插件后搜 `chrome` / `wx` / `idea`：行为与无插件时基本一致，且 **0 插件进程启动**。
- 输入 `=100*1.13`：Kite 内部 Panel 显示表达式与结果 `113`，Enter 复制；**不弹外部计算器窗口**。
- 输入 `win kite`：Kite 列表内出现窗口候选；Enter 走插件动作激活窗口。
- 插件进程被强杀或 sleep 30 秒：Kite 仍可继续搜索、Esc、启动应用。
- 快速输入 `=1` → `=10` → `=100`：过期结果不得覆盖当前结果（与既有「搜索代际」语义一致）。

## User Stories

1. 作为 Kite 用户，我希望插件能力是可安装/可卸载的，以便启动器核心保持我熟悉的轻量形态。
2. 作为 Kite 用户，我希望不使用插件时搜索 `chrome` 的结果与未安装插件时基本一致，以便扩展能力不会拖慢日常启动。
3. 作为 Kite 用户，我希望普通 Query 永远优先走 Core Search，以便 AppItem 搜索不被插件评分干扰。
4. 作为 Kite 用户，我希望只有明确 Trigger 命中时才激活插件，以便插件不会偷看我每一次输入。
5. 作为 Kite 用户，我希望输入 `=1+2` 能在 Kite 内看到 `3`，以便计算器体验完整留在启动器里。
6. 作为 Kite 用户，我希望计算器结果出现在 Kite Panel 而不是外部窗口，以便操作路径短且不抢焦点。
7. 作为 Kite 用户，我希望 Panel 里显示「Enter 复制结果」，以便我知道下一步该按什么。
8. 作为 Kite 用户，我希望 Enter 复制走 Kite 原生能力，以便复制行为与其他结果一致。
9. 作为 Kite 用户，我希望窗口切换插件把候选显示在原生列表里，以便我用同一套键盘操作选择窗口。
10. 作为 Kite 用户，我希望 Enter 激活所选窗口，以便 `win` 交互与应用启动习惯一致。
11. 作为 Kite 用户，我希望 Keyword Trigger 不会误触发（如 `tr` 不会命中 `tree`），以便普通搜索不会被打断。
12. 作为 Kite 用户，我希望 Prefix Trigger 命中后进入明确的 Provider Mode，以便当前输入由哪个插件负责是清楚的。
13. 作为 Kite 用户，我希望 Provider Mode 下不再与 Core App/File/其他插件评分竞争，以便结果稳定、可预期。
14. 作为 Kite 用户，我希望 Trigger 条件消失或我按 Esc 时退出 Provider Mode，以便随时回到应用搜索。
15. 作为 Kite 用户，我希望隐藏 Kite 后再呼出时插件状态不会卡死，以便中断路径安全。
16. 作为 Kite 用户，我希望插件默认不运行，只有真正用到时才启动，以便资源占用低。
17. 作为 Kite 用户，我希望同一插件的多个命令/提供器复用同一个进程，以便不出现「一个功能一个后台进程」。
18. 作为 Kite 用户，我希望连续输入插件 Query 不会每次都拉起新进程，以便快速输入仍然流畅。
19. 作为 Kite 用户，我希望插件长时间不用后进程自动退出，以便低常驻目标不被破坏。
20. 作为 Kite 用户，我希望安装 100 个未触发插件时仍是 0 插件进程，以便扩展数量不等于后台负担。
21. 作为 Kite 用户，我希望插件崩溃时 Kite 不退出，以便故障被隔离。
22. 作为 Kite 用户，我希望插件卡住时 UI 仍可输入和启动应用，以便 hang 不等于启动器死机。
23. 作为 Kite 用户，我希望插件反复崩溃后当前会话不再自动拉起它，以便避免崩溃循环浪费资源。
24. 作为 Kite 用户，我希望 Plugin Manager 显示插件状态并支持启用/禁用/重载/打开目录/看日志/卸载，以便我能管理已安装能力。
25. 作为 Kite 用户，我希望不兼容的插件被明确标记且不会被硬跑，以便升级安全。
26. 作为 Kite 用户，我希望插件列表结果的 `priority` 只影响该 Provider 内部排序，以便它不能改写我的应用排名。
27. 作为插件开发者，我希望用 `plugin.json` 声明 id、版本、运行命令与贡献点，以便不依赖私有格式。
28. 作为插件开发者，我希望 Command 在插件未启动时也能被搜索到，以便用户能通过标题/关键词进入能力。
29. 作为插件开发者，我希望 V1 只暴露 Command 与 Provider 两个贡献点，以便协议范围可控。
30. 作为插件开发者，我希望 Trigger 仅支持 prefix 与 keyword，以便我可以精确控制何时被调用。
31. 作为插件开发者，我希望通过 stdio JSON-RPC 与宿主通信，以便技术栈无关（任意可执行程序）。
32. 作为插件开发者，我希望 Query 请求带有 generation，以便宿主能丢弃过期响应。
33. 作为插件开发者，我希望可选地声明 cancellation 能力，以便优化取消路径（但不声明也不影响正确性）。
34. 作为插件开发者，我希望 Query 响应只要返回声明式 List/Panel 数据，不必自己建 UI。
35. 作为插件开发者，我希望 Panel 只使用少量原生 Block（text/value/key_value/notice/divider），以便结果在 Kite 中渲染稳定。
36. 作为插件开发者，我希望可以用 NativeAction（copy_text/open_url/open_path）完成常见动作，不必为此写 RPC。
37. 作为插件开发者，我希望复杂操作用 PluginAction 走 `plugin/execute`，以便仍能扩展行为。
38. 作为插件开发者，我希望 Window 作为 PluginAction 打开，而不是一种特殊 QueryResponse，以便协议更干净。
39. 作为插件开发者，我希望 Host API 仅保留极少量能力（clipboard.write / open_url / open_path / hide_kite），以便边界清晰。
40. 作为插件开发者，我希望日志写在 stderr，stdout 只走协议，以便帧不会被日志污染。
41. 作为插件开发者，我希望 Manifest 中的相对路径必须落在插件目录内，以便不能借插件执行任意路径。
42. 作为插件开发者，我希望 Plugin ID 全局唯一（推荐 reverse domain），以便多插件共存不冲突。
43. 作为插件开发者，我希望 plugin version / schema_version / plugin_api 三者分离，以便兼容判定不混乱。
44. 作为插件开发者，我希望 V1 禁止 DLL/cdylib 注入，以便我用任意语言写独立 exe 也能接入。
45. 作为维护者，我希望 Stage 0 先把结果模型泛化成 Result + ResultAction + ResultSource，以便 App/File/Plugin 共用一条展示-动作链。
46. 作为维护者，我希望 Result 管展示、Action 管行为，以便启动/复制/插件执行不混在 AppItem 字段里。
47. 作为维护者，我希望插件目录结构落在应用数据目录下，以便安装器与文档可预测。
48. 作为维护者，我希望插件数据目录与 Kite SQLite/Index/Cache 隔离，以便插件不能改写 Core 状态。
49. 作为维护者，我希望 Activation Router 极轻，普通搜索只增加少量 prefix/keyword 判断，以便热路径成本可控。
50. 作为维护者，我希望 Plugin RPC 不在 Iced UI 线程执行，以便插件慢时 UI 不冻结。
51. 作为维护者，我希望 Initialize/Query 有硬超时（目标更短），超时只丢弃本次结果。
52. 作为维护者，我希望 Crashed 后不自动后台重启，而是等用户下次显式使用时重试。
53. 作为维护者，我希望插件进程日志有限额并可轮转，以便磁盘不被日志吃掉。
54. 作为维护者，我希望 V1 不实现 Plugin Store/账号/在线更新，以便交付范围聚焦本地插件。
55. 作为维护者，我希望 Clipboard History 等常驻监听能力明确延后到 BackgroundService/API V2，以便 V1 不破坏低常驻目标。
56. 作为维护者，我希望官方验证插件至少覆盖 Calculator（Panel + NativeAction）、Window Switcher（List + PluginAction）、DevTools（一进程多 Provider）。
57. 作为维护者，我希望测试优先走三条接缝（路由/运行时/结果映射），以便不把正确性绑死在真实进程与截图上。
58. 作为维护者，我希望实现顺序按 Stage 0→6 推进，以便每一步都有可验收切片。
59. 作为维护者，我希望模块继续放在单 crate 的 `src/plugin/` 内，除非未来真正需要公开 Rust SDK，以便避免过早拆 crate。
60. 作为维护者，我希望 V1 明确不做：插件自定义 Ranking、改 Index、改 Kite UI、访问 SQLite、完整 Sandbox、Global Provider、Background Service，以便安全与性能承诺可兑现。

## Implementation Decisions

### 产品边界

- **什么做插件**：删除后 Kite 仍是完整 Windows Launcher 的能力。示例：Calculator、Translator、UUID、Hash、Timestamp、Window Switcher、JSON/Color/Git 工具、AI/OCR 等。
- **什么留 Core**：全局快捷键、应用扫描、应用索引、应用搜索、Alias、History Ranking、窗口、设置、Plugin Runtime、Plugin Manager。
- **交互形态**：V1 不按 PluginType 分类；交互只有 **List / Panel**（主形态）与 **Window**（复杂插件的特殊动作）。运行模式外置 ≠ UI 外置：Calculator Runtime 可以是 `calculator.exe`，用户仍只看到 Kite Panel。
- **性能承诺**：安装大量插件但未触发时，普通应用搜索与无插件时基本一致；未启动插件只保留 Manifest Registry，不加载 exe/DLL/UI/运行时资源。

### 通用结果模型（Stage 0，先于插件能力）

- 逐步把当前「结果 = AppItem 展示」的模型改为通用结果：
  - Result 承载 id/title/subtitle/icon/source/score 与展示所需字段。
  - ResultAction 独立承载行为，与展示分离。
- ResultSource 概念区分：App / File / Web / Builtin / Plugin（含 plugin_id + provider_id）。
- ResultAction 概念区分：LaunchApp / OpenFile / OpenUrl / CopyText / Plugin（plugin_id + action_id + payload）。
- **不改 Ranking 核心逻辑**；AppItem 相关个性化（MatchScore、Usage、Recency、Query History、Pin、Demote、明确匹配保护、搜索代际）继续作用于应用结果路径。
- 文件结果、网页结果、系统状态结果逐步改挂到同一 Result/Action 边界，避免 UI 为插件再开一套启动判定。

### 插件位置与身份

- 默认插件根目录：`%APPDATA%\com.kite.launcher\plugins\`
- 插件数据目录：`%APPDATA%\com.kite.launcher\plugin-data\<plugin-id>\`
- 插件不得读写：Kite SQLite、Kite Index、Kite Cache、Kite Internal Config。
- Plugin ID 必须全局唯一；推荐 reverse domain；允许 `a-z 0-9 . - _`；重复 ID 拒绝加载或标记 Incompatible。
- Manifest 统一为 `plugin.json`（复用现有 serde/serde_json，不为 Manifest 另引配置格式）。

### Manifest Schema 1

关键结构（决策性 schema，来自用户 Draft）：

```json
{
  "schema_version": 1,
  "plugin": {
    "id": "com.kite.calculator",
    "name": "Calculator",
    "version": "0.1.0",
    "description": "Calculator for Kite",
    "author": "Kite"
  },
  "compatibility": {
    "plugin_api": 1,
    "minimum_kite_version": "0.3.0"
  },
  "runtime": {
    "command": "calculator.exe",
    "args": [],
    "startup_timeout_ms": 2000,
    "idle_timeout_ms": 60000
  },
  "contributes": {
    "commands": [
      {
        "id": "open",
        "title": "计算器",
        "keywords": ["calculator", "calc", "计算器"],
        "action": {
          "type": "enter_provider",
          "provider": "calculate"
        }
      }
    ],
    "providers": [
      {
        "id": "calculate",
        "response_mode": "panel",
        "triggers": [
          { "type": "prefix", "value": "=" }
        ]
      }
    ]
  }
}
```

- `runtime.command` 允许插件目录内相对可执行文件名；**禁止** `../../evil.exe` 一类逃逸。`command` / `icon` / asset 路径一律 canonicalize 后确认位于 Plugin Root。
- `idle_timeout_ms` 请求值必须 clamp 到 **15s–300s**；默认 60s；**V1 禁止 0 = 永不关闭**。
- 版本三者分离：`plugin.version`（插件自身）/ `schema_version`（Manifest）/ `compatibility.plugin_api`（协议）。API 不匹配 → Incompatible，禁止硬跑。

### Contribution 与激活

- V1 只定义 **Command** 与 **Provider**。ContextAction / BackgroundService 不在 V1。
- **Command**：插件未启动时也可进入 Core Index 的静态入口（标题/关键词）。Kite 启动时读 Manifest 注册 Command，**不**因此启动 `plugin.exe`。
- Command Action V1：
  - `enter_provider`：进入指定 Provider Mode（如「计算器」→ calculate）。
  - `plugin_action`：直接触发插件动作（如 Clipboard Manager → open_window）。
- **Provider**：按用户输入动态返回 List/Panel 的能力；声明 `id`、`response_mode`、`triggers`。
- **Trigger V1 仅**：
  - `prefix`：如 `=`
  - `keyword`：如 `tr`，仅 `tr` 或 `tr` + 空白 后的剩余 Query 生效；`tree` 不得误触发
- **V1 禁止 Global Provider / match_every_query**。普通搜索永远优先 Core Search，避免每次按键都给多个插件发 Query。

### Provider Mode

- Trigger 明确命中后进入 Provider Mode：当前 Query 主要交给该 Provider，不与 Core Apps / Everything / 其他 Plugin 混排评分。
- 退出：Esc；Trigger 不再匹配（例如删掉 `=`）；切换 Provider；隐藏 Kite。
- Provider Mode 下插件 `priority` 只影响该 Provider 内部顺序，不影响 Core Ranking。

### 运行时生命周期

状态：

```text
Discovered → Disabled / Incompatible
启用后默认 Dormant → Starting → Ready
异常 → Faulted
```

规则：

- 一插件一 Runtime Process：多 Command/多 Provider 共用同一 `plugin.exe`。
- Lazy Start：首次 Trigger/显式 Command 动作需要进程时才 spawn。
- **禁止**每次按键 spawn；后续 Query 复用进程走 RPC。
- Idle Shutdown：空闲达到 clamp 后的 timeout 退出进程。
- Crash → Faulted；Kite 继续运行；**不自动后台重启**；下次用户显式使用可重试。
- Crash Loop：60 秒内崩溃 ≥ 3 次 → 当前 Kite Session 不再自动启动该插件；Plugin Manager 提示「插件反复崩溃」，用户可手动重新加载。
- V1 明确不做 Background Plugin：无永久常驻、无开机启动插件、无后台 Hook / 持续剪贴板监听 / 持续文件监控。

### IPC 与协议

- 传输：**JSON-RPC 2.0 over stdio**（Kite stdin → Plugin；Plugin stdout → Kite；日志 stderr）。
- Framing：**Content-Length**（`Content-Length: N\r\n\r\n{JSON}`），不用 JSON Lines（允许 payload 换行、边界更稳）。
- 命名空间：
  - 宿主调插件：`plugin/initialize`、`plugin/query`、`plugin/execute`
  - 插件调宿主：`host/clipboard.write`、`host/open_url`、`host/open_path`、`host/hide_kite`
- Initialize 携带 `plugin_api` / `kite_version` / `plugin_id` / `locale` / `data_dir`；插件返回声明的 capabilities（query/execute/cancellation）。
- Query 携带 `provider_id`、`raw_query`、`query`（有效 Query）、`generation`。
- Cancellation 可选（`$/cancelRequest`）；**Generation 是正确性保证**，取消只是优化。
- 所有 Plugin RPC 不得跑在 Iced UI 线程。
- 超时：
  - Initialize：目标 < 500ms，硬超时 2000ms
  - 本地 Query：目标 < 50ms，硬超时 800ms
  - 超时：丢弃本次结果，不得卡住 Kite
- Hang 兼容：插件 sleep 30s 时，用户仍必须能继续输入、Esc、呼出 Kite、搜应用并启动。

### 查询响应与展示

QueryResponse V1 仅：

```text
list | panel | empty
```

- Window **不是** QueryResponse；它属于 Action 实现。
- **List 结构**：`id / title / subtitle / icon / priority / action`；priority 0–100，仅 Provider 内部。
- **Panel Schema（声明式，禁止 HTML/CSS/JS/WebView/自定义 DOM）**：

```json
{
  "type": "panel",
  "blocks": [
    { "type": "text", "text": "100 × 1.13", "style": "secondary" },
    { "type": "value", "label": "Result", "value": "113", "selectable": true }
  ],
  "actions": [
    {
      "id": "copy",
      "label": "复制结果",
      "shortcut": "Enter",
      "default": true,
      "action": { "type": "copy_text", "text": "113" }
    }
  ]
}
```

- Panel Block V1 仅：`text`（style: normal/secondary/muted/error）、`value`、`key_value`、`notice`（level: info/warning/error）、`divider`。不提供任意布局、Canvas、HTML、自定义 Widget。
- 渲染链固定：`Plugin JSON → Panel Schema → Iced → Native UI`；搜索框在 Provider Mode 保留。
- 键盘：Esc 退出 Provider Mode（插件不得重写 Esc 基础语义）；存在 default action 时 Enter 执行之。

### ResultAction 与执行

- **NativeAction**（Kite 直接执行，不额外 RPC）：`copy_text`、`open_url`、`open_path`。
- **PluginAction**：`action_id` + `payload`；Kite 发 `plugin/execute`。
- Window Plugin：收到 `open_window` 后由插件自己创建/唤醒窗口（Iced/WinUI/WPF/Qt/Flutter/Tauri/Electron/Win32 皆可）；Kite 不负责 Window UI。
- Host API 使用原则：**能用声明式 ResultAction 完成的，不要调用 Host API**（Copy 优先 `copy_text`，而不是 `host/clipboard.write`）。
- 插件不可访问：AppIndex、HistoryDb、State、SearchService、SQLite Connection、Iced Widget、Window Handle 等内部对象。API 永远是明确数据协议。

### 安全模型

- V1 **不是** Sandbox。插件是普通 Windows 用户进程；不提供虚假的 network/filesystem permission 表象。
- Kite MUST：
  - 不自动提权 Plugin
  - 不做 DLL 注入、不把第三方代码载入 `kite.exe`
  - 不暴露 SQLite / 内部对象
  - 不允许 Plugin 修改 Core Index、Ranking、Kite UI
  - Plugin Crash 不影响 Kite
- 禁止第三方：DLL Plugin、Rust cdylib、动态库注入。原因：Plugin Crash/Hang/Leak ≠ Kite Crash/Hang/Leak。

### Plugin Manager 与分发

- 设置页新增「插件」：名称、版本、启用状态；支持启用/禁用/重新加载/打开目录/查看日志/卸载。
- V1 **不做** Plugin Store：本地 plugins/ 手动放入即可；无账号、市场、排行榜、评论、在线自动更新。
- 未来包形态 `.kiteplugin` = ZIP（plugin.json + exe + icon + assets）；Kite 校验→确认→安装。不在 V1 必做范围。

### 模块与内部架构（单 crate）

新增 `src/plugin/`，建议模块职责（名称级，不锁死文件布局）：

- Registry：扫描目录、Manifest 校验、Compatibility、Enable/Disable、Command/Provider 注册、Runtime State
- Activation Router：Query → 是否匹配 Trigger（必须非常轻）
- Host：Lazy Spawn、Initialize、JSON-RPC、Request Map、Timeout、Cancellation、Crash、Idle、stderr
- Protocol / Process：帧编解码与进程生命周期
- Panel：Panel Schema → 可供 UI 使用的声明式面板数据

内部数据流：

```text
Plugin Registry
      │
      ▼
Activation Router
      │
      ▼
Plugin Host
      │
      ▼
Process Runtime
```

最终查询流：

```text
Query
  ├─► Core Search（默认）
  └─► Activation Router ─► Trigger? ─► Plugin Host ─► JSON-RPC ─► Plugin
                                                              │
                                            List / Panel ◄──┘
                                                              │
                                                    Kite Iced UI
                                                              │
                                     Native Action / Plugin Action ─► (必要时) External Window
```

### 官方验证插件（V1 验收样本）

1. **Calculator**：prefix `=`，Panel；Enter 走 Native `copy_text`；不出现外部窗口。
2. **Window Switcher**：keyword `win`，List；Enter 走 `plugin_action.activate_window`。
3. **DevTools**：单插件多 Provider（uuid/hash/ts/json），验证一 Runtime 多 Provider 复用。

### 实现顺序（Stage 0–6）

- **Stage 0**：Generic Result + ResultAction（先重构，不改 Ranking 核心）。
- **Stage 1**：Manifest / Registry / Compatibility（插件还不能运行）。
- **Stage 2**：Command Contribution / Activation Router / Provider Mode（仍无真实 Runtime）。
- **Stage 3**：Process Host / JSON-RPC / Content-Length / Initialize / Execute / Timeout / Crash / Idle。
- **Stage 4**：List Response + Plugin Action（先用 Window Switcher 测）。
- **Stage 5**：Panel Schema + Panel Native Renderer + Native Action（Calculator 验证）。
- **Stage 6**：Plugin Manager UI（Enable/Disable/Reload/Logs/Uninstall）。

### 性能与内存目标

- 100 Plugins 全 Dormant：**0 Plugin Process、0 Plugin RPC**；`chrome` 等普通搜索与无插件基本一致。
- 未启动插件：内存只保留 Manifest Registry。

## Testing Decisions

### 好测试的标准

- 只断言**外部可见行为**：给定 Query/Manifest/状态，观察「是否激活、effective query、返回的 List/Panel/Empty、动作是否 Native 执行或转 RPC、进程是否按规则启动/复用/退出、过期 generation 是否被丢弃」。
- 不断言内部私有调用次数、具体日志文案、或与行为无关的模块内部结构。
- 不把「编译通过」或「某次真实 Windows 弹窗截图」当作算法/协议正确性的唯一门槛；真机验收用于集成冒烟。

### 三条约定接缝（已与用户确认）

1. **路由缝（主缝，纯函数）— Activation Router + Plugin Registry**  
   - 输入：Query 字符串 + 已加载的 Manifest 贡献（Command/Provider/Trigger）。  
   - 输出：无激活，或 `{plugin_id, provider_id, effective_query, response_mode}`。  
   - 覆盖：prefix 命中、keyword 精确边界（`tr` vs `tree`）、Command 可被搜索到且不启动进程、无 Global Provider、普通 Query 不进插件路径、Provider Mode 进入/退出条件（Trigger 不再匹配等）。  
   - 覆盖 Manifest 校验：schema/compat/plugin id 冲突/路径逃逸拒绝/idle clamp。

2. **运行时缝（生命周期）— PluginHost 门面 + 可注入 ProcessBackend**  
   - 测试用 mock 进程/假 stdio，不依赖真实 exe。  
   - 覆盖：Dormant 下零 spawn；首次激活 spawn + initialize；连续 query 复用同一 runtime；idle timeout 退出且 clamp 生效；initialize/query 硬超时丢弃结果；hang 不阻塞调用方；crash → Faulted 且 Kite 门面仍可服务；crash loop 抑制自动重启；generation 过期丢弃；cancellation 可选但不影响 generation 正确性。  
   - 覆盖 Content-Length 帧编解码与 JSON-RPC 消息形状（可作为 protocol 纯测，挂在运行时同一职责下，不新开业务缝）。

3. **结果缝（展示映射）— Plugin Response → 通用 Result / 声明式 Panel**  
   - 输入：List/Panel/Empty 响应；输出：可展示的通用结果或面板数据 + ResultAction。  
   - 覆盖：List 字段映射；Panel Block 合法类型与非法 block 拒绝；default action 与 Enter 语义；NativeAction 由宿主执行（如 copy_text 不触发 plugin/execute）；PluginAction 转 execute payload；priority 仅 Provider 内排序。

### Stage 0 的既有缝

- SearchResult 泛化后，应用搜索正确性仍走 **既有 search 统一入口**（个性化/保护/搜索代际不回退）。
- 启动有效性仍走 **既有 UI 启动判定缝**（过期列表不可启动）；Plugin 结果的 Action 判定复用同一「结果代际有效」思路，不另起一套。
- prior art：`search` 模块统一入口与服务 worker generation 测试；`history` 保护/分层不变量；UI 键盘/Alt+数字测试风格。

### 必测验收行为（对应 Draft §85）

1. 多插件安装后 `chrome`/`wx`/`idea`：不启动插件进程，结果与无插件基本一致。
2. `=1+2`：Kite 内显示 `3`，无外部 calculator 窗口；Enter 复制。
3. `win kite`：结果在 Kite 列表；Enter 走插件动作。
4. 强杀插件进程：Kite 不退出。
5. 插件 sleep 30s：Kite UI 不冻结，仍可搜索与启动。
6. 快速 `=1`/`=10`/`=100`：旧 generation 结果不得覆盖新结果。
7. 停止使用插件：达到 idle timeout 后进程退出。
8. 100 插件全 Dormant：0 Plugin Process。
9. 非法路径/不兼容 API：拒绝加载或 Incompatible，不硬跑。
10. `cargo test` 全绿；插件相关新增测试位于三条接缝，不依赖商店或网络。

### 验证命令（项目既有约定）

```powershell
cargo check
cargo test
cargo build --release
```

涉及窗口/进程/托盘真实行为时，除自动化外补 Windows 运行验证；不要把 release 编译成功直接当成插件生命周期验收。

## Out of Scope

V1 明确不做（来自 Draft §86 / 相关章节，实施时不得扩大）：

- DLL Plugin / Rust cdylib / 把第三方代码载入 `kite.exe`
- WebView Plugin、HTML/JS Plugin UI、自定义 Widget/Canvas
- Plugin Store、账号、市场、排行榜、评论、在线自动更新
- Plugin Dependencies、Plugin-to-Plugin RPC
- Background Service、永久常驻插件、开机启动插件、持续监听类能力（含完整 Clipboard History）
- Global Provider / match_every_query / AI Routing
- Plugin 自定义 Ranking、修改 Kite Index、修改 Kite UI
- 插件直接访问 SQLite 或任何 Kite 内部对象
- 完整 OS Sandbox 与权限谎言（假的 network/filesystem permission）
- V1.1 才考虑的 Trigger：`regex`、`files`（拖入文件）等
- 为插件拆出多个 Cargo crate / 公开 Rust SDK
- 改变 Everything 文件搜索协议、扫描器 Tier 分层语义、History Ranking 算法本身

不在本规格内：安装器插件商店 UI、云同步插件配置、插件签名 PKI（可后续单独规格）。

## Further Notes

- **输入来源**：用户提供的《Kite Plugin System V1》完整 Draft（Draft 状态，目标 v0.3.x，Plugin API 1，Manifest Schema 1）。本规格是对该 Draft + 当前仓库现状的综合，不新增 Draft 之外的产品能力。
- **仓库现状**：当前 `SearchResult` 仍嵌套 `AppItem`；尚无 `src/plugin/`。常驻搜索 worker 已有 generation/协作取消/最新请求覆盖语义，插件过期结果丢弃应对齐该「搜索代际」心智，而不是另发明一套全局锁。
- **领域词汇**：本功能引入的新概念建议在实现时同步补进 `docs/CONTEXT.md`（实现阶段再改文档，避免规格阶段空转）：Plugin、Provider、Trigger、Provider Mode、Command Contribution、Panel Schema、NativeAction、PluginAction、Runtime State（Dormant/Starting/Ready/Faulted/Incompatible）、搜索代际在插件 Query 上的复用。
- **ADR 关系**：`docs/adr/0001`（rusqlite 历史）与 `docs/adr/0002`（Everything 文件搜索）均不与插件系统冲突。插件不得改写 SQLite/Index 的约定与 ADR 0001 的「存储仅 Core 拥有」一致；文件类插件能力若未来出现，也不得绕过/替换 Everything 策略去自建全盘索引。
- **模块约束**：遵守 `docs/DEVELOPMENT.md` 与 `AGENTS.md`——业务进 `src/plugin/` 等职责模块；`main.rs`/`lib.rs` 只装配；启动/执行不得把用户输入拼进 shell；搜索行为变更必须补 Rust 回归测试；保持单 crate、聚焦改动。
- **发布节奏**：插件系统目标版本 v0.3.x；正式版本仍走 main 上 `v*` tag + CI 绿的既有发布流程，不在本规格内改发布制度。
- **后续可拆工单方向**（供 `/to-tickets`，非本规格必做）：Stage 0 泛化；Manifest/Registry；Router/Provider Mode；Process Host/协议；List+PluginAction；Panel+NativeAction；Plugin Manager UI；官方三插件验收。每条工单应指明主要落在哪条测试缝上。
