# 2026-11-06：TxF 内部记录导致文件索引持续重试

## 现象与证据

文件搜索显示“文件索引等待自动重试”，错误为 `D: FRN 19984723347166805 refresh failed: 参数错误。 (os error 87) (backend)`。本机日志中同一身份连续失败至少 16 次，退避已达到 300 秒。

用户授权一次只读管理员诊断后，直接调用 Windows 元数据 API 确认：

- FRN 为 `0x00470000000ad655`，ordinal 为 710229，sequence 为 71。
- `FSCTL_GET_NTFS_FILE_RECORD` 返回相同 ordinal／sequence 的在用普通文件 base record；有一个 resident FILE_NAME，无 ATTRIBUTE_LIST。
- 文件名为 `0000000000007381`，父目录 FRN 为 `281474976710687`。
- 启用 SeBackupPrivilege 后，`OpenFileById` 对文件仍返回 Win32 87；对父目录成功，父目录身份校验一致，路径为 `D:\$Extend\$RmMetadata\$Txf`。

因此这是仍存活的 TxF 内部文件，现有增量刷新无法按编号打开它。原逻辑仅能证明它仍存在，随后继续传播 87；同步轮次无法完整提交，后台每次从旧游标重读又遇到同一身份。

[Microsoft 对 FSCTL_GET_NTFS_FILE_RECORD 的说明](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_get_ntfs_file_record)要求核对返回身份：该调用会向较小 ordinal 查找在用记录，返回值可能与请求不同。诊断和实现均比较完整身份，打开失败本身不作为消失证据。

## 改动

保留原有消失身份判断。对打开失败为 Win32 87、身份仍存活的非保留文件，复用建库已有的严格 resident FILE_NAME 解析器，要求 exact ordinal／sequence／type、在用 base record、完整属性遍历、无 ATTRIBUTE_LIST、完整名称计数。按经过卷与完整身份校验的父目录生成路径；之后再次读取名称，集合变化时拒绝本轮刷新。无法证明完整性的对象继续报错。

同时实现应用结果右键“清除记录”。入口仅对本地绝对路径明确不存在的应用显示，执行时重新核对当前索引身份与路径状态。在同一 SQLite 事务中删除该身份的使用历史、查询学习、固定、降权、按身份关联的 Alias 和手动登记，并保留现有隐藏身份，阻止残留快捷方式和旧快照重新显示它。设置的“已隐藏入口”可恢复显示，已清理的历史及偏好仍保持清理状态。

本机 MiMo 条目的来源为 `start-menu`，启动目标 `D:\Program Files\Xiaomi MiMo\Xiaomi MiMo.exe` 已不存在；快捷方式解析仍保留这个启动目标，解释了手动删除软件后的残留结果。

## 验证与边界

在本机 `.data/diagnostics-20261106/` 创建隔离的只读诊断工具，直接调用原有 `journal::paths` 刷新路径；只追加诊断入口，使用相同的真实 FRN 与系统权限：

- 改动前：`refresh-before.json` 为 `ok=false`，错误 87。
- 改动后：`refresh-after.json` 为 `ok=true`，路径为 `D:\$Extend\$RmMetadata\$Txf\0000000000007381`。
- `cargo check --locked --offline` 通过。
- 复用 `cargo test --locked --offline --lib result_panel_tests`，8 项现有检查通过；本轮没有新增测试用例。

## 补充运行验证

使用 Debug 开发构建及隔离 profile，保留现用 Kite 与读取组件。应用数据库通过 SQLite backup 复制，索引基库及已提交变化状态复制到独立目录；所有写入、查询结果和截图均留在本机诊断目录。

- **三卷追平**：候选内核的完整 watcher 从现用 C/D/E 三卷的持久游标续读，生产 LiveSearch 消费真实 feed，约 10.97 秒达到 Ready，查询命中本轮新增的诊断文件。最终状态 `live=true`、`stale=false`、`rebuild_required=false`、`pending_objects=0`；本次报告累计 41,478 项刷新变化。
- **停止后续读**：停止隔离 watcher 后改名诊断文件，再次启动从上轮最终持久游标恢复，三个游标逐项完全一致；约 0.80 秒达到 Ready，查询只返回改名后的路径，最终仍实时就绪、无 stale、无待处理对象。
- **身份生命周期**：完整续读时原 TxF 身份已经自然消失，feed 对该身份输出 `identity_only=true`。前面的单记录运行对照验证了对象仍存活时的 87 处理；本次完整续读同时确认后续消失身份的正常回收。
- **右键实际点击**：隔离开发窗口查询真实残留 MiMo，菜单显示“清除记录”。在副本中补齐该身份的使用历史、查询学习、固定、降权、Alias 和手动登记后点击，条目立即消失并提示“已清除应用记录”；数据库逐表核对六类记录均为 0，隐藏身份为 1。
- **窗口重启**：结束本次开发进程，复用同一隔离 profile 重新打开，搜索 MiMo 仍不回显。现用开始菜单残留快捷方式保持原样，确认屏蔽在重新扫描和重启后持续生效。

证据为 `.data/diagnostics-20261106/` 下的 `sync-second.json`、`sync-restart.json`、`ui-records-before.json`、`ui-records-after.json` 及 `ui-context.png`、`ui-cleared.png`、`ui-restart.png`。没有安装、替换或重启现用 Kite 和读取组件，现用用户数据未修改。正式打包和安装验收按项目发布流程由 Actions 完成，长期在线观察继续作为发布后的观察范围。
