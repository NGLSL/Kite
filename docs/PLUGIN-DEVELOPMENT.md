# Kite 插件开发说明书（Plugin API v1）

面向第三方与官方插件作者。宿主实现见 `src/plugin/`；完整产品规格见 `.scratch/plugin-system-v1/spec.md`。

**协议不限制语言**：任何能读写 stdio + `Content-Length` JSON-RPC 2.0 的语言/运行时都可以写插件（Rust / C# / Go / Python / Node…）。

**官方插件用 Rust**（`official-plugins/`）：出于性能与启动开销考虑，不是协议强制。Rust 作者可选用 `kite-plugin-sdk`；其他语言自行实现同等帧协议即可。

---

## 1. 定位与原则

| 原则 | 含义 |
|------|------|
| 能力外置 | 插件是**独立进程**；崩溃/卡顿/泄漏不得拖垮 Kite |
| 体验内联 | 结果在 Kite 原生列表/面板展示；不弹抢焦点的「插件 UI」（Window 动作除外） |
| 显式激活 | prefix/keyword Trigger 命中时只在普通搜索列表显示插件入口；选中入口后才启动进程并展示结果 |
| 可卸载 | 删除插件目录后 Kite 仍是完整 Windows Launcher |

**适合做插件**：删除后 Kite 仍是启动器的能力（计算器、窗口切换、UUID/Hash、翻译、AI…）。

**应留在 Core**：应用索引与启动、URL 直达与网页搜索槽位、文件搜索集成、历史/排序/热键/托盘等「启动器本体」。

---

## 2. 包结构与 Manifest

一个插件 = 一个目录（安装到 `%APPDATA%\com.kite.launcher\plugins\<plugin-id>\`）：

```text
com.kite.calculator/
  plugin.json          # 必填
  kite-plugin-calculator.exe   # runtime.command 指向的可执行文件（或任意语言入口）
```

`runtime.command` 可以是 `.exe`，也可以是 `.cmd` / `.bat` / 已关联解释器的脚本启动器；**相对插件根目录**，禁止 `..` 与绝对路径。

### plugin.json

```json
{
  "schema_version": 1,
  "plugin": {
    "id": "com.kite.calculator",
    "name": "Calculator",
    "version": "0.1.0",
    "description": "…",
    "author": "…"
  },
  "compatibility": {
    "plugin_api": 1,
    "minimum_kite_version": "0.3.0"
  },
  "runtime": {
    "command": "kite-plugin-calculator.exe",
    "args": [],
    "startup_timeout_ms": 2000,
    "idle_timeout_ms": 60000
  },
  "contributes": {
    "commands": [
      {
        "id": "open",
        "title": "计算器",
        "keywords": ["calc", "计算器"],
        "action": { "type": "enter_provider", "provider": "calculate" }
      }
    ],
    "providers": [
      {
        "id": "calculate",
        "response_mode": "panel",
        "triggers": [{ "type": "prefix", "value": "=" }]
      }
    ]
  }
}
```

```json
{
  "schema_version": 1,
  "plugin": {
    "id": "com.kite.calculator",
    "name": "计算器",
    "version": "0.1.0",
    "description": "在搜索框输入 = 直接计算，支持括号与优先级",
    "usage": "在搜索框输入 = 加上算式即可计算。\n\n示例\n  =1+1\n  =1+1*(2+1)\n\nEnter 复制结果。",
    "author": "Kite"
  },
  "compatibility": {
    "plugin_api": 1,
    "minimum_kite_version": "0.3.0"
  },
  "runtime": {
    "command": "kite-plugin-calculator.exe",
    "args": [],
    "startup_timeout_ms": 5000,
    "idle_timeout_ms": 60000
  },
  "contributes": {
    "examples": ["=1+1", "=1+1*(2+1)"],
    "commands": [
      {
        "id": "open",
        "title": "计算器",
        "keywords": ["calc", "calculator", "计算器"],
        "action": { "type": "enter_provider", "provider": "calculate" }
      }
    ],
    "providers": [
      {
        "id": "calculate",
        "response_mode": "panel",
        "triggers": [{ "type": "prefix", "value": "=" }]
      }
    ]
  }
}
```

约束：

- `plugin.id`：`[a-z0-9._-]+`，推荐 reverse-domain，≤128。
- `schema_version` / `plugin_api` 必须为 **1**。
- **`plugin.name` / `plugin.description` 必填**：说明用用户语言写清「能做什么、在搜索框怎么触发」；空说明宿主拒绝加载（Incompatible）。
- **`plugin.usage` 强烈建议**：多行详细用法；设置 → 插件点「说明」展示。缺省时宿主用 description/triggers/examples 自动拼装。
- **至少一条可发现入口**：`contributes.examples`、`commands` 或带 `triggers` 的 `providers` 三者必居其一。
- **`contributes.examples` 强烈建议**：设置页试用芯片与搜索空态提示读这里；缺省时从 Trigger/Command 推导。
- `runtime.command`：**相对插件根目录**；禁止 `..`、绝对路径、盘符。文件须真实存在。
- `idle_timeout_ms`：默认 60000，宿主 clamp 到 **15s–300s**；`0` 不表示永不退出。
- `startup_timeout_ms`：默认 **5000**；宿主硬上限 5000ms（manifest 默认同步）。
- `response_mode`：`list` | `panel`（无结果时返回 `empty`）。
- Trigger：`prefix`（如 `=`）或 `keyword`（如 `win`；仅整词或词+空白边界，`tree` 不会命中 `tr`）。
- V1 **无 Global Provider**：未命中 Trigger 的 Query 不会调用插件。

设置 → 插件会展示 `description` 与 `examples`，点示例即填入搜索框试用；无说明/无入口的包不会进入可用列表。

---

## 3. 进程模型与生命周期

```text
Dormant ──(首次 Trigger/Command 需要进程)──► Starting ──initialize ok──► Ready
   ▲                                                    │
   │         idle timeout / reload / 禁用               │ crash / spawn 失败
   └────────────────────────────────────────────────────┴──► Faulted
```

- **一插件一进程**：多 Command/多 Provider 共用同一 runtime。
- **Lazy Start**：普通搜索不 spawn；未 query 的 100 个插件 = 0 进程。
- **复用**：Ready 后连续 Query/Execute 走同一进程 RPC。
- **Idle Shutdown**：空闲超过 clamp 后的 timeout，宿主杀进程 → Dormant。
- **Crash**：标记 Faulted；**不自动后台重启**；下次用户显式使用可重试。
- **Crash Loop**：60s 内崩溃 ≥3 次 → 本 Kite 会话不再自动启动该插件。
- **硬超时**：Initialize ≤2000ms；Query/Execute ≤800ms；超时丢弃本次结果，Kite 不卡死。
- **Generation**：查询代际；过期结果必须丢弃（取消 RPC 可选，代际是正确性保证）。
- 所有插件 RPC **不得**跑在 Iced UI 线程（宿主已在线程池/工作线程处理）。

---

## 4. IPC 协议

- **传输**：JSON-RPC 2.0 over **stdio**（与实现语言无关）  
  - 宿主 → 插件：插件 **stdin**  
  - 插件 → 宿主：插件 **stdout**  
  - 日志：插件 **stderr**（宿主写入 `plugin-data\<id>\stderr.log`，256KB 轮转）
- **Framing**：`Content-Length: N\r\n\r\n{UTF-8 JSON}`（**不要** JSON Lines）

### 宿主 → 插件

| method | params | result |
|--------|--------|--------|
| `plugin/initialize` | `plugin_api`, `kite_version`, `plugin_id`, `locale`, `data_dir` | `{ plugin_api, capabilities: { query, execute, cancellation } }` |
| `plugin/query` | `provider_id`, `raw_query`, `query`（有效 Query）, `generation` | 见 §5 |
| `plugin/execute` | `action_id`, `payload` | 任意 JSON（如 `{ok:true}`） |
| `$/cancelRequest` | `{ id }` | 通知，可选 |

宿主亦可能发送 `plugin/dispose`（请求 id 通知/应答均可）；插件收到后应尽快退出或清空本地状态。

### 插件 → 宿主（Host API，尽量少用）

| method | params |
|--------|--------|
| `host/clipboard.write` | `{ text }` |
| `host/open_url` | `{ url }` — 仅 http/https |
| `host/open_path` | `{ path }` — 仅盘符绝对路径或 UNC |
| `host/hide_kite` | `{}` |

有 `id` 的 host 请求宿主会回 `{ ok: true }`。  
**能用声明式 NativeAction 就不要调 Host API**（复制优先 `copy_text`）。

---

## 5. Query 响应

### Empty

```json
{ "type": "empty" }
```

### List（Provider Mode 整列表交给插件，不与 Core 混排）

```json
{
  "type": "list",
  "items": [
    {
      "id": "window-1",
      "title": "Visual Studio Code",
      "subtitle": "可选",
      "icon": "可选，相对插件根",
      "priority": 10,
      "action": {
        "type": "plugin_action",
        "action_id": "activate_window",
        "payload": { "hwnd": 123 }
      }
    }
  ]
}
```

- `priority`：0–100，**仅 Provider 内部排序**，不影响 Core 排序。
- item `action.type`：
  - `plugin_action` + `action_id` + `payload` → 宿主发 `plugin/execute`
  - `copy_text` / `open_url` / `open_path` → Native，不 RPC
- 宿主用当前 `plugin_id` 填充，插件无需写 `plugin_id`。

### Panel（声明式，禁止 HTML/WebView/自定义控件）

```json
{
  "type": "panel",
  "panel": {
    "blocks": [
      { "type": "text", "text": "100 × 1.13", "style": "secondary" },
      { "type": "value", "label": "Result", "value": "113", "selectable": true },
      { "type": "key_value", "items": [{ "key": "SHA", "value": "…" }] },
      { "type": "notice", "level": "error", "text": "…" },
      { "type": "divider" }
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
}
```

- `text.style`：`normal` | `secondary` | `muted` | `error`
- `notice.level`：`info` | `warning` | `error`
- 存在 `default: true` 时，Enter 执行该 action。
- **Panel DTO 不接受 `launch_app`**：插件不得启动 Core 索引应用。

---

## 6. Action 语义

| 类型 | 谁执行 | RPC |
|------|--------|-----|
| **NativeAction** `copy_text` / `open_url` / `open_path` | Kite | 否 |
| **PluginAction** `action_id` + `payload` | 插件（`plugin/execute`） | 是 |
| **Host API** `host/*` | Kite（能力极少） | 是（插件发起） |

安全闸门（宿主强制）：

- `open_url`：仅 `http://` / `https://`
- `open_path`：仅 `C:\…` 或 `\\server\share\…`；拒绝 `shell:`、`ms-settings:`、元字符
- 路径逃逸的 `runtime.command` 在扫描期标记 Incompatible

**插件不可访问**：AppIndex、HistoryDb、SQLite、Iced Widget、Window Handle 等内部对象。

---

## 7. 官方插件（Rust，性能选择）

仓库位置：`official-plugins/`（**独立 workspace**，不编入 `kite` 主 crate）。

官方实现选用 Rust 以便控制延迟与体积；**第三方无需跟随**。

| 包 | Trigger | 行为 |
|----|---------|------|
| `kite-plugin-sdk` | — | 帧编解码 + `serve_loop`（Rust 可选依赖） |
| `kite-plugin-calculator` | `=` | Panel，Enter 复制 |
| `kite-plugin-window-switcher` | `win` | List，Enter 激活窗口 |
| `kite-plugin-devtools` | `uuid`/`hash`/`ts`/`json` | 单进程多 Provider；`hash` 和 `json` 入口打开宿主独立工具窗 |

构建并打包到 `resources/official-plugins/`：

```powershell
.\scripts\build-official-plugins.ps1
```

安装到用户目录：

1. 运行 Kite → 设置 → 插件 → **重装官方**  
2. 或「导入」粘贴插件文件夹路径  
3. 或手动复制到 `%APPDATA%\com.kite.launcher\plugins\`

### Rust 骨架（可选 SDK）

```rust
use serde_json::json;

fn main() {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = kite_plugin_sdk::serve_loop(
        &mut out,
        |_provider, query| {
            if query.trim().is_empty() {
                return kite_plugin_sdk::empty_response();
            }
            kite_plugin_sdk::panel_response(json!({
                "blocks": [{ "type": "value", "value": query }],
                "actions": [{
                    "id": "copy", "label": "复制", "default": true,
                    "action": { "type": "copy_text", "text": query }
                }]
            }))
        },
        |_action, _payload| Ok(json!({"ok": true})),
    );
}
```

第三方插件可 path/git 依赖 `official-plugins/kite-plugin-sdk`，或用任意语言实现同等 stdio 协议。

---

## 8. 安装、管理与调试

| 能力 | 入口 |
|------|------|
| 官方插件 | **随安装包内置**（`resources/official-plugins`）；**首次启动自动**拷入用户插件目录（已有 id 不覆盖） |
| 导入本地文件夹 | 设置 → 插件 → 粘贴路径 → 导入 |
| 重新安装官方插件 | 设置 → 插件 → **重装官方**（覆盖导入） |
| 能力说明与试用 | 设置 → 插件：点「说明」展开详细用法；点示例填入搜索框 |
| 启用/禁用/重载 | 插件列表开关与按钮 |
| 打开目录 / 日志 | 列表按钮；或打开 plugins / `plugin-data\<id>\stderr.log` |
| 卸载 | 列表「卸载」（仅允许删 plugins 根下目录） |

用户侧发现路径（产品要求，不只是开发者文档）：

1. 搜索框占位：有启用插件时提示 `=1+1 · win · uuid` 等示例  
2. 无结果空态：「也可试试：…」  
3. 设置 → 插件：一句话说明 +「说明」点开详细用法 + 示例试用  

不在主界面底部堆 toast。

注意：「内置」= 安装包携带 + 启动时 seed 到用户目录；插件仍是**独立进程**，不链入 `kite.exe`。

调试建议：

- 插件日志只写 **stderr**，勿污染 stdout（stdout 是协议通道）。
- 协议自测：向 stdin 写入 Content-Length 帧，检查 stdout 帧（语言无关）。
- 普通 Query 不应启动插件：可用 Plugin 状态或进程列表观察。
- 宿主单元测试可用 mock 进程；真机验收需真实可执行文件。

---

## 9. 验收清单（作者自测）

- [ ] `plugin.json` 校验通过；`command` 在包内且可执行  
- [ ] initialize 返回 `plugin_api: 1` 与 capabilities  
- [ ] Trigger 命中先显示插件入口；选中入口后才有插件 query；普通词无进程
- [ ] List/Panel/Empty 形状正确；非法 block 会被宿主拒绝  
- [ ] Enter：Panel default / List plugin_action 行为符合预期  
- [ ] 快速连续 query：旧 generation 不覆盖新结果（宿主侧）  
- [ ] 杀进程后 Kite 不退出；再触发可恢复或 Faulted 可手动重载  
- [ ] 空闲后进程退出  
- [ ] `open_url`/`open_path` 不传非法 scheme/路径  

---

## 10. 版本与边界

- Plugin API / Manifest Schema：**v1**  
- V1 **不做**：Plugin Store、云配置、签名 PKI、DLL/cdylib 注入、WebView/HTML UI、Background Service、Global Provider、插件改 Core 索引/排序  
- 未来包形态 `.kiteplugin`（ZIP）未纳入 V1 必做  
- 官方插件实现语言（Rust）不属于协议约束  

术语与领域模型见 `docs/CONTEXT.md`。
