# 09: 官方验证插件 + V1 验收

**What to build:** 提供官方验证插件样本并跑通规格中的 V1 验收：Calculator（Panel + NativeAction）、Window Switcher（List + PluginAction）、DevTools（单 Runtime 多 Provider）。在真实 Windows 上确认：普通搜索零打扰、进程隔离、代际丢弃、idle 退出、crash/hang 不拖垮 Kite。

**Blocked by:** 06 — List + PluginAction（Window Switcher 切片）  
**Blocked by:** 07 — Panel + NativeAction（Calculator 切片）  
**Blocked by:** 08 — Plugin Manager UI

**Status:** ready-for-agent

- [ ] Calculator：`=1+2` 在 Kite 内显示结果；Enter 复制；无外部窗口
- [ ] Window Switcher：`win kite` 等结果出现在 Kite 列表；Enter 可激活窗口（环境具备窗口时）
- [ ] DevTools：一个插件进程服务 uuid/hash/ts/json 等多个 Provider
- [ ] 多插件安装后 `chrome`/`wx`/`idea` 不启动插件进程，体验与无插件基本一致
- [ ] 强杀插件进程：Kite 不退出；Faulted 可恢复策略符合规格
- [ ] 插件 hang（sleep）：Kite UI 不冻结
- [ ] 快速 `=1`/`=10`/`=100`：过期结果不覆盖新结果
- [ ] 停止使用后 idle timeout 进程退出
- [ ] 大量插件全 Dormant：0 Plugin Process
- [ ] 全量 `cargo test` 通过；真机验收记录写入本票 Comments

## Notes

- 对应规格 Stage 官方验证 + §85 验收清单。
- 验收插件可放在仓库资源或开发目录中，不要求 Plugin Store。
- 若真机上某项因环境缺失（如无 Everything/无目标窗口），在 Comments 记录，不得静默跳过可自动化项。
