# Kite 下一版实施验收

状态：01–06 功能实现完成，07 集成验收部分通过、桌面及安装项待续。用户已进一步授权新增版本并发布，源码版本已更新为 v0.5.0；本文记录发布准备时的本地证据，线上发布状态以对应 GitHub CI/Release 记录为准，不据发布成功补勾尚未执行的实机项。

## 验收口径

- 实施起点：`4f39415687eab4d0cad684bd041d20088cf3bd6e`（仓库 v0.4.3）。
- 安装环境原程序：`D:\Program Files\Kite\kite.exe`，实际文件版本为 0.4.2；不能作为源代码 v0.4.3 的性能基线。
- 项目现有搜索评估、HistoryDb 行为接口、UI 消息更新与扫描归并是自动验证入口；真实 Windows 操作和安装版验收另行记录。
- 运行资料保留于 `artifacts/v050-verification/`，性能与评估日志保留于 `artifacts/`；这些本机资料不进入产品安装包。

## 已验证的基线

- 独立 detached worktree：`C:\Users\admin\AppData\Local\Temp\kite-v043-baseline-4f39415`，固定于上述起点提交。
- release 性能报告：`artifacts/v043-perf-baseline.txt`，`report_perf_baseline` 通过，1 passed。
- 固定搜索评估：`artifacts/v043-search-eval.txt`，53 样本，Top1 / Recall@5 / MRR 均为 1.000。
- 精确源代码基线 exe：`artifacts/v050-verification/kite-v043-baseline.exe`，文件版本 0.4.3，保留用于同口径运行比较。

## 搜索与性能对比

- 最终搜索 fast path 修正后的固定 53 样本：Top1 / Recall@5 / MRR 均 1.000，P50 / P95 为 132 / 239 µs；日志 `v050-search-eval-final.txt`。此前同机基线为 139 / 233 µs，尾部耗时有运行噪声，不能把单轮差异直接解释为用户感知退化。
- 发现并处理无隐藏项时的额外 HashSet / 组遍历：首次三轮空查询 800 项 P50 为旧版 21 µs、候选 31 µs。加入空 hidden 快路径后，交替运行旧版与候选各三次，中位 P50 / P95 为旧版 22 / 24 µs、候选 21 / 22 µs。12 项 hidden、21 项 empty_query 与热缓存定向回归通过，独立 Spec 审核确认非空 hidden 语义不变。
- 最终 800 项常用查询 P50 旧版 → 候选：chrome 55 → 56、vis 44 → 44、微信 5 → 5、weixin 58 → 57 µs。80 项无命中 zzzz 的最终 P50 为 65 → 77 µs、P95 83 → 101 µs；此前三轮同案例为 54 → 49 µs，不是持续同方向退化。保留全部数据，不宣称每种查询都更快。
- 三轮明细与中位值：`v043-perf-final-1..3.txt`、`v050-perf-final-1..3.txt`、`v050-perf-final-comparison.json`。性能工具标题仍硬编码旧的 0.3.7 字样，实际二进制分别来自固定 v0.4.3 起点与当前候选，不把标题当作版本证据。

## 自动检查

- 存储定向检查：`cargo test storage --lib`，47 passed；包含 v2 → v3 迁移、隐藏重载、查询清理保留其他数据、多表身份迁移失败回滚及快捷方式当前身份刷新。
- 实际 COM 快捷方式、Bootstrap 手动扫描、同身份归并定向测试通过；watcher 定向测试 6 passed，验证纯手动父目录非递归、与原有根重叠时保留原监听模式，以及已注册根随后加入 Portable 时重新注册递归模式。
- 搜索 hidden / demote / pinned 定向回归通过；`hidden_preference_replays_on_hot_base_cache_without_invalidation` 通过，隐藏/恢复使用相同热缓存重放最新偏好。
- `cargo check` 通过；最终 `cargo test`：622 passed，0 failed，11 ignored；主程序与文档测试通过。忽略项保留项目既有人工/性能验证口径，性能报告另行运行。
- 全套测试首先发现旧 Pin 测试使用无数据库状态却要求成功刷新；已补真实 HistoryDb 并断言固定写入，定向通过后全套复验通过。
- 新增空查询、设置页打开时 `ManualRemove → FullIndexReady → CloseSettings` 消息回归，验证首页采用新快照且已移除结果不可再启动。
- 新文件 rustfmt 检查与全部 staged/worktree `git diff --check` 通过；保留未涉及区域原有格式。
- 最终 `scripts/build-installer.ps1` 通过，包含 `cargo build --release`、官方插件 workspace 14 项测试及 NSIS 构建。实机候选运行锁曾导致一次 exe 覆盖失败，停止对应候选后重建成功；不是编译错误。

## 本地 v0.5.0 产物（发布准备时）

| 产物 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `target/release/kite.exe` 与 `artifacts/kite.exe` | 10432512 | `73C25F380AFBAA102858AF8B9FD32FE04F9793C67D3FD44B71EF2A4B6995CBF7` |
| `artifacts/kite-setup.exe` | 5436765 | `33530A5EE83D793549F14F9C654A87AFD0B43237B465812C3664F1FF04CC5877` |

PE FileDescription / ProductName 均为 `Kite`，文件版本为 0.5.0。主程序与安装器打包的 exe 哈希一致，版本清单见 `artifacts/v050-local-release-artifacts.json`；`plugin-layout-artifacts.json`、`candidate-artifacts-final.json` 保留此前 5DA/D72 的历史清单。GitHub 发布安装器由独立 CI 环境构建，其大小及 SHA 以线上构建报告和独立下载校验为准，不混用本地哈希。最终安装后的运行路径/哈希另行核对，不能据打包文件直接宣称安装版通过。此前 A69F/D72/5DA 的证据不改记为 v0.5.0 实机证据。

## 版本发布准备

- 按用户明确指令发布 v0.5.0，版本与 Cargo.lock 已同步；现有功能提交先 fast-forward 到 dev，在 dev 准备版本后再将相同候选推进 main，不重写历史。
- 版本后的 `cargo check`、全套 `cargo test`（622 passed / 11 ignored）及 `build-installer.ps1` 通过，PE 版本和名称核对一致；本轮日志为 `v050-release-check.txt`、`v050-release-tests.txt`、`v050-versioned-installer-build.txt`。
- 维护者发布说明在 `docs/releases/v0.5.0.md`，独立只读复核升级/回退和验证措辞，无阻止发布的错误声明。
- 仅在 main 精确候选提交的 CI 成功后创建 annotated tag；GitHub Release 由该 tag 构建，完成后独立下载安装器对比报告 SHA。线上成功不替代下面保留的 Windows 实机验收项目。

## 插件页布局追加优化

- 用户截图中开关实为 60×32，原因是 40×22 内部控件叠加 Iced Button 默认 padding；显式 `.padding(0)` 后恢复 40×22，共用设置开关保持一致。
- 插件卡片的示例与「说明 / 重载」并为同一底行，取消管理动作独占行。长示例在留给示例的宽度内换行，管理按钮保持独立宽度；动作消息和插件执行流程不变。
- 用实际 `settings_view`、官方插件 manifest、系统 Noto Sans SC 字体及 tiny-skia Headless 渲染器，按截图相同的 718×518 尺寸生成深浅色预览并人工检查。每张官方卡片从 139.1 px 降为 101.2 px，减少约 27%；三张卡片完整显示，下方维护区可见。另检查长中英文示例自动分行且按钮无裁切。
- 证据保留于 `artifacts/`：`plugin-layout-before.txt`、`plugin-layout-after.txt`、`plugin-layout-long.txt`、`plugin-layout-after-dark.png`、`plugin-layout-after-light.png`、`plugin-layout-long-dark.png` 和 `plugin-layout-long-light.png`。这是原生代码的离屏渲染，未称实际安装窗口截图；临时采证模块已移除，不增加产品依赖或永久测试。
- `cargo check`、97 项 UI 回归、最终全套 622 passed / 11 ignored 均通过；`build-installer.ps1` 再次完成 release、14 项官方插件测试与 NSIS，最新安装器 5436862 字节。没有覆盖当前用户安装或执行安装器，没有推送/发布。

## Standards

首次独立规范审核发现 3 项问题，修复后逐项复核均 closed：

- 面板隔离遗漏 Alt+数字文本 fallback：共同消息路径现在禁止面板态快捷启动，原触发消息回归通过。
- 手动 CRUD 回空查询首页采用旧快照：成功写入后标记明确重扫请求，Full 完成时采用快照并刷新；新增移除消息闭环回归通过。
- 手动父目录后来加入 Portable 后仍保持非递归：记录已监听模式，配置改变时 unwatch / rewatch，实际协调逻辑回归通过。

## Spec

首次独立规格审核发现 3 项问题，修复后逐项复核均 closed：

- 面板中的 Alt+数字隔离不完整：同上，规格要求的消息路径已覆盖。
- Alias 文本修改后过期冲突确认可能覆盖另一映射：修改输入清除旧冲突，替换再次确认 Alias、旧目标 ID / 名称及 IME 状态；过期确认不改变映射的回归通过。
- 新增 Alias / 隐藏权限误收紧已有 Pin / Demote：权限分开判断，保留内置 / 系统 / 插件既有动作；系统从最新索引解析，可信内置命令保持原契约；实际固定写入回归通过。

两个维度各 3 项，原最严重问题均为 P1，复核无剩余阻塞发现。审核针对实施起点至当前工作树，未将用户已有 skills 更新当作本轮产品改动。

## 提示消失后的焦点回归

- 实际采证曾报告 Alias 保存后的输入焦点丢失，但复核后该旧缺陷未被证实。初步记录中的“点击也无效”受输入工具组合键时序影响，修正工具并准确匹配 `query="…"` 日志后，旧候选两轮点击输入均通过；此前 `focus-red-52d.json` 的 false 是日志匹配误报，不能当产品红证据。
- 后续复核 `focus-corrected-52d` 和 `candidate-final-focus` 的编辑截图发现，脚本点击坐标落在菜单外，实际是取消编辑；固定 Alias 已存在，陈旧 DB receipt 不能证明本轮保存。因此这两批 `directQueryPassed=false / clickedQueryPassed=true` 不构成 Alias 保存后的红证据，原失焦现象尚未稳定确认。
- 最小反馈修正为：每轮用此前不存在的新 Alias；以最新窗口截图中实际保存按钮坐标操作，保存前后 DB receipt 和成功 Toast 共同证明本轮写入，再分别验证直接输入与点击输入。最终 `focus-valid2-a69f.json` exit 0，两轮 before receipt 均为空、after receipt 为正确目标，`directPassed=true / clickedPassed=true`，截图与日志一致；主 Agent 复核实际 query 为 focusfixture。
- 单变量候选调整固定 Toast 的外层 Stack；提示出现/消失只替换第二个子节点，不再替换搜索输入所在子树。未保留未经验证的延后聚焦或 sleep 方案。Standards 复核 Iced 0.14.2 Tree diff、Space 事件和布局后无阻塞；Spec 复核符合保存后恢复焦点/保留 Query 的契约。
- A69F 对应源码全套测试 622 passed / 11 ignored，该候选真实保存后的输入两轮通过。该结果验证 A69F 候选的保存路径可用；由于旧候选“红反馈”取证错误，不声称已证明一个稳定旧版缺陷被修复。
- 最后的 D72 候选补齐外点取消 Alias 编辑的焦点恢复：仅编辑态且没有工具窗打开时恢复主输入焦点。Iced 的 widget operation 会跨窗口遍历并 unfocus 其他控件，不能无条件聚焦；Spec 指出这项风险后已窄化，Standards 独立复核无 blocker。97 项 UI 测试和全套 622 / 11 再次通过，安装包重新构建成功。这一新增实际点击路径尚待桌面复验，A69F 保存后的两轮证据不能代替。

## Windows 实机、安装版与升级

实际 Windows 验收进行中；安装器实际运行需要 Windows 管理员提权。

- 当前 Codex 由 EnvBox 启动，普通子进程会继承注入。已通过现有 Explorer 启动未注入的 host helper，核对进程没有 EnvBox 模块；原用户数据在首次测试前备份，未注入运行的 Windows 行为另行取证。
- 恢复中发现注入与未注入进程对相同 KnownFolder 路径确有不同目录观察结果，证据 `host-recovery-counts.json` 含无 EnvBox 模块的进程证明。当前 Aura 源码与文档未实现文件系统 overlay/whiteout，不能用静态源码否定当前已加载 runtime 的实测差异，也不能据此命名隔离机制；原因尚未确认。最早原用户备份为 832 文件；其后未注入基线启动形成的默认测试数据备份为 428 文件。最终按实测逐步恢复最早原用户数据，各中间快照另存，不混合数据库快照。
- 实际 v2 数据库迁移到 v3 后，旧设置、Alias、Pin、Usage、Query 配对与 Demote 共 6 张表逐行核对保留，证据 `candidate-v3-seed-preserved.json`。
- 同一物理主机、相同 325 个应用索引的稳定空闲采样：各等待 15 秒，再采集 20 次。源代码 v0.4.3 基线平均工作集 56.75 MiB、私有内存 30.40 MiB；此前 52D 候选分别为 51.96 MiB / 27.55 MiB；约 19.2 秒内 CPU 增量均 0.125 秒。这不是最终 D72 二进制的空闲采样，本次未观察到增加，不据单次顺序测量宣称性能优化。
- 工作区候选实际完成 Shift+F10 与右键同一动作列表、Alias 保存后即时搜索，以及冲突取消/确认替换；隐藏后重启仍过滤，再从设置恢复，原 Alias、Pin、Usage 和 Query 配对保留。这些操作已有开发候选证据，最终安装版仍需另验。
- 最终 A69F 候选经 Kite 实际启动夹具：EXE `argv=[]`、cwd 为 `C:\Users\admin`（沿用既有 home 默认）；LNK `argv=[--fixture-mode, shortcut, 静态 参数]`、cwd 为中文/空格目录 `夹具 有空格\工作 目录`。严格时间边界后的 receipt 见 `candidate-launch-Exe-receipt.json` / `candidate-launch-Shortcut-receipt.json`，不能以单独自检替代 Kite 启动。
- 单 Query 实际清理/重学：`db-before-forget.json` → `db-after-forget.json` 只移除 focusfixture 的 count=2 配对；其他 Query、Usage、Settings、Alias、Pin、Demote、Hidden 和 Manual 全部一致。再次成功启动后 `db-after-launch-Exe.json` 中 focusfixture count=1。主 Agent 对这些导出副本独立断言通过。
- 管理页实际完成两条 EXE/LNK 登记、EXE 改名、同路径重复登记不增行、目标暂失时不可用、恢复重扫后可用、移除 EXE 登记并保留原 exe/lnk 文件哈希、重新登记供安装重载验证。缺文件期间搜索中原身份不再可启动的实际证据仍待安装版补充，不能用管理状态/总数代替。
- Microsoft 拼音实际物理按键产生候选条；Enter 提交组合文本时 Alias 编辑仍开启，Alias/Query/Usage DB 不变；Space 实际选字为“你好”后取消。证据 `ime-behavior-proof.json`、`candidate-ime-chinese-selected.png`，Unicode SendInput 不作为 IME 验收。
- Base64 固定长文本的双栏编码、换行与滚动已实际打开验证，主 Agent 复核 `candidate-base64-long-columns.png`；Hash / JSON 尚未完成，不能合并称三个工具通过。其他 Codex 会话切换/关闭工具窗口后 foreground guard 拒绝输入，已暂停桌面操作并协调空闲时间。
- 采证工具曾直接以 SQLite `mode=ro` 打开中间 428 文件的默认测试数据备份，仍改写了 `kite-history.db-shm`。其中仅该共享内存文件哈希改变，主 DB、WAL 与其余 427 文件保持；原 expected / actual 保留在 `host-backup-hash-recheck.json`。该快照全部另存且明确记录例外，不删除、重建 SHM，不把它当作原用户 832 文件备份。
- 恢复脚本先回放 428 数据，随后向已经存在的目标 `Move-Item`，导致最早 Roaming 备份嵌套进目标目录，严格计数检查中止。所有原件保留；修正为先把当前目录整体移到经检查的唯一 sibling，再把原备份移回确实不存在的原路径。Local 移动后仍观察到另一进程可见的 416 个默认测试文件；未向现存目标回放备份，另由未注入 helper 把该目录完整移到同父唯一 sibling，并核对前后 SHA，再确认原路径不存在并恢复 716 个原件。禁止把不同快照的 DB/WAL/SHM 混拷。
- 原用户备份共 832 文件，内容 SHA-256 全部与原记录相同。Roaming WAL 初始 inventory 的长度为 2974672，而后续完整 FileStream 实际读取 3118872 字节；该完整流 SHA 仍与原记录完全相同（`nested-original-wal-read-proof.json`）。这是清单长度不一致，未发现内容哈希改变；保留初始记录与独立差异，不改 expected、不称长度全部严格一致。所有 SQLite 语义采证此后只打开副本。
- 最终路径恢复与启动前复核完成：`injected-restored-prestart-hashes.json` 中 Roaming 116 + Local 716 = 832 个文件，全部 SHA 相同，extraFiles=0，唯一上述长度例外。主 Agent 独立逐项比较原 inventory 与实际证据的全部名称/SHA，通过 832 项；这是启动前证明，正常程序启动后的日志或数据库写入不混作还原差异。
- 原安装版 0.4.2 已正常后台重启，PID 32632，路径 `D:\Program Files\Kite\kite.exe`，SHA `D24B83B7A571DBA75901C6B84A573C379F6947563CE83EEB2F15E54B349FF551` 不变。`original-installed-restarted.json` 核对前台 HWND/PID 均未变化；未提权、未切换前台，候选与测试插件无残留。测试文件及各快照保留，安装器尚未实际执行，注册表/快捷方式未因安装测试修改。

操作前备份实际 KnownFolder 数据；结束后逐文件核对原数据 SHA-256 并恢复原程序。安装测试限定独立候选目录，原安装目录文件保持；注册表和快捷方式备份、安装与恢复在同一个提权 helper 中完成。

## 未验证项

- 最终产物的安装器实际执行、受控覆盖安装、安装后运行路径/哈希和各功能的安装版复验；Windows UAC 尚未启动。
- 最终 v0.5.0 二进制的稳定空闲采样及外点取消 Alias 后实际输入；已有空闲数据来自之前的 52D 候选，保存后焦点证据来自 A69F。插件页离屏预览不代替安装版窗口交互。
- 目标缺失期间搜索中旧 EXE/LNK 身份不能再启动的实际证明；已有管理页不可用/恢复证据不能代替此项。
- Hash / JSON 双栏长文本的真实视觉检查；Base64 已通过。
- 多屏和不同 DPI 环境：本机仅一个 1920×1080、100% 缩放显示器，未擅自改变用户显示配置。
- 当前注入与未注入进程目录观察差异的根因未确认；当前任务仅完成必要数据恢复，不扩展修改 Aura 或删除测试快照。

上述桌面与安装动作等待用户确认空闲时段；自动测试、搜索质量评估和已有真实操作不因此改称未验证。任何尚无证据的项目都不得据此标记完成。
