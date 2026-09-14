Status: ready-for-agent
Type: task
Blocked by: 01 后台完整扫描与原子索引快照、02 入口变化监听与刷新合并、05 图标链路正确性、09 排序分层与 History 学习

## Goal

在真实 Windows 上对完整搜索体验做端到端验收：安装/卸载/快捷方式变更/重复入口/历史排序/图标显示均以可见结果与进程稳定为准；编译通过不算完成。

## Scope

- 开始菜单、桌面、Scoop、App Paths 的安装/卸载/改名；目标：最后一次入口变化后约 5 秒内看到稳定结果。
- 更新过程中搜索可用；一次安装事件不产生持续重扫。
- 同机可启动入口覆盖抽样：深层目录、带参数快捷方式须进入索引；未入索引的须能指出过滤原因。
- 图标覆盖率：系统入口、普通 exe、快捷方式、Scoop、UWP、常见文件类型。
- NVIDIA App、Razer Synapse、Git Bash/CMD/GUI、多个 PowerShell 入口纳入端到端样本（作一般回归，不写产品特例）。
- 记录超时与原因；对照票 06 基线报告指标变化。
- 沿用 `cargo test` 与 `cargo build --release`。

## Acceptance Criteria

- [ ] 运行中安装应用后无需重启即可搜到；卸载后失效入口消失。
- [ ] 重命名快捷方式后 Query 使用新名称。
- [ ] Scoop shim 新增/移除后结果自动变化。
- [ ] App Paths 注册变化后可搜到无快捷方式程序。
- [ ] 同程序多来源不占据多个相邻重复位；不同参数/独立功能入口仍保留可启动。
- [ ] 图标与当前安装状态一致（含 CC Switch、控制面板类样本）。
- [ ] 多个 PowerShell 入口身份与参数分别核对，点击后窗口保持运行。
- [ ] `visual code`/`vs code`/`todo`/`ter`/`vsc`/`ndm`/`雷蛇` 等样本达到规格期望。
- [ ] `cargo test` 与 `cargo build --release` 通过；实机检查项有记录。

## Validation

- `cargo test`
- `cargo build --release`
- Windows 实机验收清单（安装/卸载/改名/图标/召回/History/延迟）逐项打勾，结果追加到本票 Comments 或同目录报告。

## Dependencies

- Blocked by: 01、02、05、09
- 建议 03、04、06、07、08 已合入，以便一次验收覆盖身份稳定性与召回样本。

## Out of Scope

- 发布、签名、版本号、推送、合并到 main 的流程决策。
- 与第三方启动器的强制对比排名。

## Comments
