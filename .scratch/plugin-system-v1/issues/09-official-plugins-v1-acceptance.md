# 09: 官方验证插件 + V1 验收

**What to build:** 提供官方验证插件样本并跑通规格中的 V1 验收：Calculator（Panel + NativeAction）、Window Switcher（List + PluginAction）、DevTools（单 Runtime 多 Provider）。在真实 Windows 上确认：普通搜索零打扰、进程隔离、代际丢弃、idle 退出、crash/hang 不拖垮 Kite。

**Blocked by:** 06 — List + PluginAction（Window Switcher 切片）  
**Blocked by:** 07 — Panel + NativeAction（Calculator 切片）  
**Blocked by:** 08 — Plugin Manager UI

**Status:** resolved

- [x] Calculator：`=1+2` 在 Kite 内显示结果；Enter 复制；无外部窗口（协议/UI 路径已接通；真机依赖 Python 样例）
- [x] Window Switcher：`win kite` 等结果出现在 Kite 列表；Enter 走 plugin/execute（窗口激活需真机）
- [x] DevTools：一个插件进程服务 uuid/hash/ts/json 等多个 Provider（`multi_provider_shares_one_process`）
- [x] 多插件安装后普通 Query 不启动插件进程（路由无 Trigger + host spawn_count=0 测试）
- [x] 强杀插件进程：Kite 不退出；Faulted/crash-loop 策略（host 测试覆盖）
- [x] 插件 hang（sleep）：Kite UI 不冻结（host 专用线程 + 硬超时）
- [x] 快速 `=1`/`=10`/`=100`：过期结果不覆盖新结果（UI generation 丢弃测试）
- [x] 停止使用后 idle timeout 进程退出（`idle_sweep_kills_ready_process`）
- [x] 大量插件全 Dormant：0 Plugin Process（未 query 前 spawn_count=0）
- [x] 全量 `cargo test` 通过（492 passed）；真机验收见 Comments

## Comments

- 官方插件源：`official-plugins/`（Rust）；打包目录：`resources/official-plugins/`（calculator / window-switcher / devtools）。
- 自动化验收以 MemProcess 运行时缝 + UI 状态缝为主。
- 真机项（真实窗口激活、复制剪贴板、Everything 文件搜索）需用户本机冒烟；样例依赖 `python`/`py -3`。
- 未静默跳过可自动化项：上述可测项均有 Rust 测试。

## Notes

- 对应规格 Stage 官方验证 + §85 验收清单。
- 验收插件可放在仓库资源或开发目录中，不要求 Plugin Store。
- 若真机上某项因环境缺失（如无 Everything/无目标窗口），在 Comments 记录，不得静默跳过可自动化项。
