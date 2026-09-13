Status: resolved
Type: task
Blocked by: 01

## Goal

为搜索结果增加日常使用所需的上下文操作。

## Scope

- 增加右键菜单和键盘菜单入口。
- 支持打开所在文件夹。
- 支持复制路径和复制显示名称。
- 支持固定和取消固定结果。
- 通过明确的 item id 与 action 调用 Rust IPC。
- 对普通应用、文件、文件夹、UWP 结果分别处理。

## Acceptance Criteria

- 应用结果可以打开其所在目录。
- Everything 文件和文件夹结果可以打开所在位置。
- 复制操作不依赖用户输入拼接命令。
- 固定项可以取消固定，并在空 Query 中稳定排序。
- 菜单支持鼠标和键盘，Esc 或点击外部可以关闭。
- 找不到目标或动作失败时显示可理解的错误，不导致窗口崩溃。
- 不改变 Enter 启动、上下选择和 Alt+1～9 行为。

## Validation

- `cd src-tauri; cargo test`
- `npm run build`
- Windows 实机检查普通应用、UWP、文件和文件夹四类结果。
- 验证复制内容、目录定位和固定排序。

## Dependencies

- 依赖 issue 01 的共享 IPC 和设置缓存稳定后再合入。

## Out of Scope

- 以管理员身份运行；可作为后续独立任务。
- 任意命令执行。
- 完整剪贴板历史。

## Comments

2026-09-13 实现记录：

- 新增 `result_action` IPC（id + action：`open_folder` / `pin` / `unpin`），
  open_folder 在 Rust 侧按 id 解析索引目标（file: 结果按路径），目录直接打开、
  文件用 opener 插件 reveal（资源管理器定位），无 shell 拼接。
- 复制路径 / 复制名称为纯剪贴板写，在前端完成（`shared/clipboard.ts`，
  navigator.clipboard 优先、execCommand 兜底），无命令拼接。
- 右键（contextmenu）与键盘（Menu / Shift+F10）均可呼出菜单；
  菜单内 ↑↓ / Enter / Esc 可操作，capture 阶段拦截避免 Esc 顺带隐藏窗口；
  点击外部关闭。Enter 启动、上下选择、Alt+1～9 在菜单关闭时行为不变，
  Alt+1～9 在菜单打开时仍可用。
- 固定（Pin）：新增 `pinned` 表与 `list_pinned` IPC；空 Query 固定项排在
  最近使用之前（pinned_at 降序），非空 Query 获得固定加分——与历史加分
  取较大者、不叠加，保证 Prefix+固定(800+180) 仍低于 Name Exact(1000)。
- 分类处理：普通应用 / 文件 / 文件夹可「打开所在文件夹 + 复制路径」；
  网址与浏览器搜索项可复制 URL；UWP / 系统设置 / 内置项只保留复制名称与固定。
- 失败反馈：目标缺失或动作失败经 StatusRegion 显示错误，不影响窗口。
- 验证：`cargo test` 127 通过（含 pin 排序/保护、动作目标识别用例）；
  `npm run build` 通过。待人工实机确认四类结果（普通应用 / UWP / 文件 / 文件夹）的
  菜单动作与固定排序。
