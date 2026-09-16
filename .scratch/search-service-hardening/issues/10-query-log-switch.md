# 10: 查询级日志开关

**Status:** resolved
**Blocked by:** None (can start immediately)
**Type:** performance / maintainability

**What to build:** 查询级的诊断日志（结果就绪、结果过期等）可以被关掉，关掉之后按键路径上不再产生同步的文件写入；默认仍然保留可诊断性。

- [ ] 应用结果完成／过期等查询级日志受一个明确开关控制
- [ ] 关闭后，按键路径上不产生同步日志写入
- [ ] 默认状态仍保留可诊断性，不静默掉全部日志
- [ ] 开关不改变任何搜索结果、顺序或缓存行为

## Notes

- 现状已经比「每个结果写几十条」收敛很多，但每次查询仍有若干条同步 `info` 级写入，仍在按键路径上。
- 目标不是删日志，而是让它可控：需要排查时打开，日常关闭。

## Answer（提交 7d45454、a2763f9、902ff48）

开关落在**内存**里（`State.query_log`），只在启动与打开设置页时从 SQLite 读一次
（`storage/settings.rs` → `ui/runtime.rs` / `ui/actions.rs`）：按键路径上绝不查库，
否则「省一次文件写」会被「多一次 SQLite 读」抵掉。

```text
Settings.query_log（默认 true，持久化）
  → State.query_log（内存）
     → State::qlog(|| ...)  （ui/mod.rs，与 plog 同处）
        → 关闭时直接返回：不构造消息、不落盘
```

- `qlog` 接 `FnOnce() -> String`，关闭时**连消息都不构造**，所以不只是不写文件，也没有
  字符串格式化的开销。日志文案在关掉时不再产生。
- 受控的调用点（窗口内交互路径，全部 `plog` → `qlog`）：
  - 按键频率：`alt down` / `alt up` / `ime composing` / `ime commit`
    （`ui/interaction.rs`）、`app search ready` / `app search stale` / 空 Query 刷新
    （`ui/results.rs`）、`file search ready` / `file search stale`、`alt-digit text
    fallback idx=`（`ui/interaction.rs`，与 `app search` 同属查询级，7d45454 漏掉）。
  - 一次性动作：`alt-n idx`、`key named ... ignored`、Esc（`ctx menu closed` /
    `hide issued`）、Enter 启动与 `launch ...`、右键菜单三处（`ctx open_folder` /
    `ctx pin toggle` / `ctx demote`）、`files toggle`、`settings open/close`
    （`ui/actions.rs`），以及设置窗口内由键盘/指针触发的 `open releases` /
    `open repository` / `autostart` / `hotkey change rejected` /
    `hotkey unavailable`、`rescan requested`、`quit requested`
    （`ui/interaction.rs`）。
- 仍走 `plog`（不受控）的是**不由窗口内输入触发**的那些：窗口/热键生命周期
  （`show`/`hide`/`blur`/二次激活/`window ready`）、后台子系统（引导、扫描、索引
  构建、entry watch、图标、字体、托盘、更新下载线程、`full index ready`、
  `identity remap`）。它们不随按键重复，也不落在窗口内输入处理栈上。
- 默认状态仍保留可诊断性：开关默认 `true`，关闭是用户的显式选择。

### 测试

- `query_log_off_never_builds_the_message`：关掉后 `qlog` 不得构造消息
  （闭包里放 `unreachable!`）。**已做变异验证**：去掉闸门即 panic 失败。
- `query_log_on_still_emits_by_default`：默认开启时照常发射，保留可诊断性。
- `query_log_switch_does_not_change_results_or_cache`：同一查询在开/关两态下结果内容、
  顺序、代际、`results_stale` 与缓存代际/非空性完全一致。
- `set_query_log_toggle_leaves_search_state_untouched`：切换开关不重跑搜索、不动代际与缓存。
  **已做变异验证**：让处理器顺手 `refresh_results()` 即失败。
- `query_log_off_leaves_input_handlers_working`：关掉日志后 IME/Alt 输入处理照旧。
- `settings.rs`：默认 `true`、`"0"` 往返、重启后仍生效。
- `interactive_log_gate_tests`（提交 a2763f9 / 902ff48，`ui/actions.rs`）：
  驱动整条交互路径（Esc、未识别具名键、Alt+数字、启动被拒与启动设置页、设置页开合、
  文件模式切换、文件搜索落地、快捷键变更被拒），关掉开关时断言**本线程零条日志记录**，
  开启时断言上述每条都仍写出。**已做变异验证**：补闸门前该测试为红（实测输出正好列出
  当时的 5 条裸 `plog`）。为此在 `src/log.rs` 加了 `#[cfg(test)]` 的按线程日志记账
  （`LOG_PATH` 是进程级全局量，并行测试无法靠重定向文件观测写入）。

### 未验证项

- 设置页开关的实际点击、以及**真实文件写入**是否消失，仍需 Windows 真机冒烟：测试里
  `LOG_PATH` 未设置，所以只证明到「`log::info` 未被调用」——而记账点就设在 `info` 内、
  写盘之前，故这一步已足以排除按键路径触发文件追加。
- 本机为「编译 + 测试」层：`cargo test` 需临时 stub `build.rs`（无 `rc.exe`），
  已在提交前还原；`cargo clippy` 在该 toolchain 未安装，改用编译器告警把关（零告警）。

## Comments

### 代码复核采纳项

- 复核指出**字面验收项 (b) 不成立**：只闸 3 处查询日志时，`alt down/up`、`ime composing/commit`
  这 4 处**按键频率**的日志仍在同步写盘。已把这 4 处纳入同一个闸门，并顺手把 `qlog`
  从 `results.rs` 移到 `mod.rs` 的 `plog` 旁边（两个模块共用，放在结果模块不合适）。
- 仍未受控的是 Enter 启动、Esc 关闭、Alt+数字、右键菜单这类**一次性动作**日志
  （`ui/actions.rs` 的 `alt-n idx`、`ctx menu closed`、`launch ...` 等）。
  这是刻意划的边界，不是遗漏：它们不随按键次数重复，留着才有排查价值。
  若要严格字面口径，把它们一并纳入同一闸门即可（约 5 个点，无需新机制）。

### 字面口径补全（提交 a2763f9、902ff48）

上一节最后一条的「刻意边界」经讨论后**不再保留**：既然验收项 (b) 写的是「按键路径上
不产生同步日志写入」，就把它做成字面成立，而不是靠注释解释。

- a2763f9：把 `ui/actions.rs` 的 16 处交互日志（右键菜单三处、`alt-n idx`、
  Esc 两处、`key named ... ignored`、启动被拒/空结果、Everything 下载、web 启动成功与
  失败、`hide issued (launch)` 两处、app 启动成功与失败）以及 `settings open/close`
  改走 `qlog`；同时补上 7d45454 遗漏的**查询级**日志
  （`file search ready/stale`、`alt-digit text fallback`）与 `files toggle`。
- 902ff48：对 a2763f9 的复核指出该提交自己声明的「every interactive-path log」
  名不副实——设置窗口内由键盘/指针触发的 `open releases` / `open repository` /
  `autostart` / `hotkey change rejected` / `hotkey unavailable` / `rescan requested` /
  `quit requested` 仍在同步写盘。已一并纳入，边界改为：**窗口内用户事件处理栈上的日志
  走 `qlog`；窗口/热键生命周期与后台子系统走 `plog`**。`qlog` 的文档注释同步改写。

### 复核记录（两轴，基点 fb5908c）

- **Standards 轴**：无硬性文档标准违规（已核 `AGENTS.md`、`docs/DEVELOPMENT.md`、
  `docs/CONTEXT.md`、`README.md`；仓库无 `rustfmt.toml`/`clippy.toml`/`CONTRIBUTING.md`）。
  一条判断项：`qlog` 收 `FnOnce() -> String`，纯字面量被迫写 `.to_owned()`，与
  `format!` 站点形态不一致。**未采纳**：闭包只在开关开启时求值，关闭时该分配根本不发生，
  没有行为代价；为它再开一个 `qlog_str` 只会多一套 API 面。
- **Spec 轴**：①四个验收项均成立，唯 (c) 的**意图**与旧 Answer 的划界理由有出入——
  现行为是「默认开启即保留可诊断性」，而非「一次性事件不静默」；②无实质 scope creep
  （纳入一次性动作正是本票 Comments 自己给出的口径）；③指出 (b) 在设置窗口侧不成立，
  即 902ff48 所修的内容。
  顺带记录：`src/log.rs` 的测试钩子属为新增测试服务的支撑设施，非产品行为改动。
