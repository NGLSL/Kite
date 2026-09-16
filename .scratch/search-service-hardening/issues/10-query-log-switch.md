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

## Answer（提交 7d45454）

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
- 受控的调用点（按键频率）：`alt down` / `alt up` / `ime composing` / `ime commit`
  （`ui/interaction.rs`）、`app search ready` / `app search stale` / 空 Query 刷新
  （`ui/results.rs`）。
- 一次性事件日志**故意不受控**：激活/显示/隐藏、启动、扫描、索引、图标、更新、托盘、
  设置页开合。理由是工单 Notes 的定位是「每次查询仍有若干条同步 info 级写入」——
  要收掉的是按按键频率重复的那一类；把一次性事件也静默掉会牺牲默认可诊断性。

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

### 未验证项

设置页开关的实际点击、以及关掉后按键路径真的不再产生文件写入，都需要 Windows 真机冒烟；
本机只能证到「关掉后不构造消息、不调用 `plog`」这一层。

## Comments

### 代码复核采纳项

- 复核指出**字面验收项 (b) 不成立**：只闸 3 处查询日志时，`alt down/up`、`ime composing/commit`
  这 4 处**按键频率**的日志仍在同步写盘。已把这 4 处纳入同一个闸门，并顺手把 `qlog`
  从 `results.rs` 移到 `mod.rs` 的 `plog` 旁边（两个模块共用，放在结果模块不合适）。
- 仍未受控的是 Enter 启动、Esc 关闭、Alt+数字、右键菜单这类**一次性动作**日志
  （`ui/actions.rs` 的 `alt-n idx`、`ctx menu open/closed`、`launch ...` 等）。
  这是刻意划的边界，不是遗漏：它们不随按键次数重复，留着才有排查价值。
  若要严格字面口径，把它们一并纳入同一闸门即可（约 5 个点，无需新机制）。
