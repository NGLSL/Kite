# Kite Linux 适配与 Loci 独立文件搜索方案

日期：2026-10-02。阶段：方案与隔离原型验证，**尚未实现 Linux MVP，也未接入 Kite**。当前仓库基线 `8aa7ae7727f3affe216d96967b9279085f6e1a64`，起始工作树干净。

## 建议与决策边界

建议保留 Rust 原生桌面路线，Kite 使用 Iced/tiny-skia CPU 软件渲染；将文件名／路径搜索实现为独立的 **Loci** Rust 库，Kite 负责产品、桌面集成与执行动作。Linux MVP 必须带 Loci 文件搜索；不把安装 Everything、plocate、FSearch 或 root 服务当作前提。Windows 暂时保持既有 Everything 后端。

Loci 实际开发／验证目录为 **`D:\Project\Loci`**，与 Kite 并列。`Kite/experiments/loci` 保留最初实验源码、脚本与测试快照，不能把它误当作主程序依赖。用户已确认远端项目名 NGLSL/Loci；本轮不克隆、不初始化 git、不提交、不推送、不创建 PR，不写 LICENSE，也不对独立项目承诺许可证。Loci 目录在创建前不存在，本轮创建采用不覆盖的方式。

Rust **1.99.0 已安装且实际用于 Loci**，`rust-toolchain.toml` 项目级固定，manifest 的 `rust-version = "1.99"`。官方发布说明确认 1.99.0 于 2026-10-01 发布。[Rust 官方说明](https://doc.rust-lang.org/releases.html#version-1990-2026-10-01) 没有改全局默认；Kite 当前默认编译器仍为 1.97.1。Kite 升级至 1.99 是后续接入阶段的明确工作，本轮不修改其生产工程。

“普通查询首屏 100 ms、引擎常驻 100 MB”是评估用探索目标，不是硬验收线或已实现承诺。引擎常驻、建库峰值、Kite UI 总内存和操作系统缓存必须分别量；CPU 软件渲染不等于低 CPU 或整个产品低于 100 MB。

## 当前架构与 Windows 绑定

读取了根 AGENTS.md、README、Cargo.toml／Cargo.lock、DEVELOPMENT、CONTEXT、PERFORMANCE、PLUGIN-DEVELOPMENT、ADR 0002、本地 `.agents/skills` 的研究／模块设计说明，以及相关实现。该调查以源码职责为依据，未尝试让完整 Kite 在 Linux 编译。

| 范围 | 现状证据 | 后续 Linux 工作 |
|---|---|---|
| 装配／构建 | `src/lib.rs` 启动先降权、singleton；`build.rs` 资源处理已有 cfg(windows)，但 winres／windows 依赖尚未按 target 隔离；CI 仅 windows-latest | Windows 依赖与实现按 target 组织；Linux singleton／activation 通道；独立 Linux CI 与包装流程 |
| 应用扫描 | `src/app/scanner/` 使用注册表、COM、.lnk、Known Folders、App Paths；`src/app/uwp/`、control_panel、windows_settings 直接绑定 Windows | Linux 扫描 XDG `.desktop`；保留已有搜索、拼音、排序、SQLite；一个坏入口或权限错误不能中断扫描 |
| 启动／路径 | `src/app/launcher.rs` 导入 windows CommandExt、Windows terminal；`src/app/actions.rs` 只识别盘符／UNC、explorer `/select` | 明确 `LaunchTarget` 变体、参数向量与 `OsString`；验证索引目标；Linux 绝对路径／非 UTF-8 路径；文件动作使用固定系统接口，搜索输入不进入 shell |
| 文件搜索 | `src/system/everything.rs` 提供 EverythingClient、EverythingHit、FileFilter、Availability；`src/ui/results/file_search.rs`／`mod.rs` 与 recovery 仍使用 Everything 状态／source 名 | 提取产品中立的 FileSearch 接口与 freshness 状态；保留 NativeEverything adapter；Linux 接入 Loci，不模拟“未安装 Everything”状态 |
| 查询调度 | `src/search/service.rs` 已有最新请求槽、单 worker、协作取消、缓存代际；UI 有 file_query_generation 与 index_generation | 复用调度原则，让文件查询也检查取消并拒收旧 query/index generation；应用检索与百万文件索引分开 |
| 文件监听／快照 | `src/app/watch.rs` 的 notify 与注册表监听耦合；`src/app/atomic_file.rs` 的非 Windows 退路先删目标再 rename | Linux 目录 watcher 独立；快照同目录临时文件、fsync、原子 rename、目录 fsync；不能把“先删再 rename”当作 Linux 崩溃安全方案 |
| 桌面服务 | `src/system/{hotkey,singleton,window_place,icons,theme,sound,autostart,env,elevation}.rs` 与 `src/ui/{runtime,tray,font}.rs` 有 Win32／Windows 生命周期 | 将热键、激活、定位、托盘、图标／字体、开机启动、音效、主题／休眠重连放入平台职责模块；设置页按能力显示 |
| 插件／发布 | stdio 进程模块已经 cfg Windows 隐藏控制台；manifest、官方包、window-switcher 与更新／NSIS 有平台语义 | calculator／devtools 先验证 Linux 打包；Linux 不默认发布 Windows 窗口切换器；二进制平台标识、可执行权限、安装路径、更新包分别设计 |

`src/main.rs`／`lib.rs` 继续只做装配。核心检索／存储无需为了 Linux 重写；但包含 Windows 路径、source、args 字符串的 model／序列化数据需要平台区分。Linux 身份与路径不能照搬 Windows 不区分大小写的去重规则；跨系统导入设置时，不直接执行另一平台的 target。

### CPU 渲染配置

Cargo.lock 锁定 `iced 0.14.0`、`iced_renderer 0.14.0`、`iced_tiny_skia 0.14.1`。Kite 目前 `default-features=false`，显式启用 tiny-skia、crisp、thread-pool、image、canvas、unconditional-rendering，未启用 wgpu，也没有 x11／wayland。

Linux 适配时保留这些 CPU 渲染特性，并在 Linux 条件依赖显式加 `x11`、`wayland`；不能恢复 Iced 全部 defaults，因为 defaults 包含 wgpu。Iced 官方 0.14 manifest 明确软件渲染与后端特性独立。[Iced 官方特性](https://github.com/iced-rs/iced/blob/0.14.0/Cargo.toml) 后续用 cargo tree 检查 Linux 特性合并与 wgpu 缺席，再在两个会话实测开窗、输入法、HiDPI、CPU、休眠和唤起。现有 unconditional-rendering 的 CPU／刷新代价也要实测；本轮不改配置、不把依赖特性检查当作 GUI 验收。

### X11 / Wayland 支持范围

| 能力 | X11 | 原生 Wayland |
|---|---|---|
| 全局热键 | XGrabKey；处理键被占用、布局及锁定修饰键 | GlobalShortcuts portal 运行时探测、用户绑定与拒绝；无可用 backend 时提供桌面自定义快捷键启动 `kite --toggle` 的说明，安装器不改系统设置 |
| 显示／激活 | EWMH 请求，WM 仍可拒绝焦点请求 | 有效 activation token 与合成器策略；不能保证任意后台抢焦点；Iced／winit token 路径待实机验证 |
| 精准窗口定位 | 可请求位置并实测多屏缩放 | 通用 toplevel 没有任意全局坐标定位；接受合成器布局，不承诺鼠标所在屏精准居中；layer-shell 属可选特定桌面适配 |
| 托盘 | 取决于桌面支持 | StatusNotifierItem／桌面扩展支持各异；托盘缺席仍应可启动、打开设置和退出 |
| 窗口切换插件 | 可另行实现与验证 | 通用协议不提供枚举和激活任意其他应用窗口；Linux MVP 不承诺此功能 |

协议、权限与具体官方出处见 [来源研究](linux-file-search-sources.md#6-x11-与-wayland不能统一承诺窗口控制)。XWayland 的 XGrabKey 不应被宣称为所有原生 Wayland 应用的全局快捷键支持。建议验收 X11 一个明确 WM、GNOME Wayland、KDE Wayland；wlroots 必须标明具体 compositor／portal 组合，不能只写“支持 Wayland”。

## Loci 独立项目职责

| 归属 Loci | 归属 Kite |
|---|---|
| 文件／目录记录、原始路径字节与稳定 ID、根目录身份 | 应用 `.desktop`／Windows 入口扫描与启动 |
| 有界扫描、目录排除、权限失败、监听事件规范化与对账 | Iced CPU 渲染、快捷键、托盘、窗口与设置 UI |
| 名称／路径／扩展名查询、取消、分页、索引 generation／freshness | 应用拼音／模糊匹配、历史个性化、固定、混排与交互 |
| 持久快照、增量／tombstones、压缩／缓存、格式版本／恢复 | 打开文件／定位目录动作、图标／预览和安全执行 |
| 独立 CLI、基准、测试、公开的搜索语义与资源限制 | Windows Everything adapter 与 Linux Loci adapter |

建议先单 crate 内清楚划分职责，提供库与测试 CLI，不立即拆成很多 crates 或常驻 daemon。库嵌入调用开销低、可共享查询 worker；CLI 为可复现和其他调用者服务。以后若需要隔离崩溃、跨语言客户端或多个进程共享索引，再引入可选 IPC／daemon；它增加常驻进程、协议、权限和生命周期成本，不能无证据地作为 MVP 默认。

建议的外部接口（设计草案，原型没有稳定库接口）：

```text
open(config) -> Engine
query(Query { terms, field, extension, root, case_mode }, page_limit,
      cancel_token, expected_index_generation) -> Page
Page { hits: [RecordId, PathRef, kind], cursor, count_status,
       query_generation, index_generation, freshness }
refresh(scopes) -> ScanJob / Progress
status() -> roots / coverage / watcher_health / freshness
```

将新增／删除／重命名、事件合并、格式和缓存隐藏在引擎内；不要让 Kite 直接操作 posting list 或 watcher descriptor。查询 generation 每实例持有，跨实例不会互相取消。第一页的部分计数明确标 `AtLeast`／`Unknown`，完整计数及排序另行有界执行；稳定 cursor 绑定快照 generation，代际变化返回明确重查状态。

索引目标是文件名与路径，不是全文内容。MVP 用 AND 子串、名称／路径范围、扩展名、目录／根范围，中文按明确规范匹配；不先承诺 regex、Everything 全语法、中文拼音、跨平台全盘 journal 或全文内容。大小写搜索策略与文件系统身份策略分开；原始路径不被小写／Unicode 归一化覆盖，Linux 非 UTF-8 路径可保留字节并提供展示回退。

## 推荐的索引路线与取舍

首选“共享目录结构／字符串池＋不可变快照＋trigram 候选＋精确验证＋有界 delta”。原型只验证此路线的一部分，不能直接作为生产实现。

1. 目录以 parent ID 与名称组成，记录持有 parent ID／name offset／kind／稳定 ID；避免每条记录存完整绝对路径或 AppItem 的所有搜索／图标字段。目录重命名更新目录节点，不重写整棵树的路径。原型只是共享完整父路径字符串，还不是目录树。
2. 比较文件级 postings 与 16／64／256 条块级 postings：文件级选择性强、列表大；块级省空间、假阳性多。原型只测 64 条块和未压缩 u32 postings，不能直接认定该块大小最优。生产考虑 delta-varint／分块跳表／bitmap 与稀有列表先交集；以基准和语料决定压缩及块大小。
3. 词长按**规范化 UTF-8 字节**处理。单汉字可能已有 3 字节 trigram；1／2 字节 ASCII 退化问题单独解决。可比较字符／byte summary、稀疏 bigram postings、名称有序前缀范围与时间预算扫描；短查询无命中也需要有界延迟。不能靠延迟触发搜索掩盖短词缺陷。
4. 扩展名、文件／目录、root 与目录范围作为结构化约束，尽早减少候选。不同 token 在同一块中的不同文件上命中时，最终验证能保正确性，却仍可能接近全扫描；评估按文件或字段建立索引解决。
5. 不预先为全部文件存拼音、模糊删除词、图标或完整 path-normalized 副本；仅物化首屏路径与图标。结果排序定义有限质量层＋稳定 tie break；第一页不能用截断前 50 条证明全量 TopK 正确。
6. 快照先使用可校验的磁盘文件，读取策略比较局部 buffered read 和只读分段 mmap。mmap 的文件驻留页、页表、解压缓存、候选堆、delta、watch 表都消耗内存；其映射大小不能当作 RSS，private bytes 也不能替代 RSS／PSS。[Linux 内存口径来源](linux-file-search-sources.md#8-mmap-与内存口径)

SQLite 留给 Kite 的配置／历史及少量控制元数据，不把百万文件逐行全量加载成 String／AppItem；本轮没有测 SQLite FTS 或拿结果证明其一定更慢。对比方案可以保留为后续测项。

## 扫描、更新、丢事件与恢复

Linux MVP 默认普通用户、显式选定 roots（产品可推荐 home 子目录，由用户选择），排除缓存、构建产物和虚拟文件系统，默认不跟随 symlink、不越挂载点、不扫描 `/` 或其他用户。错误计数和不可访问目录在状态中可见，部分覆盖不能声称全盘完整。

先用 inotify，每目录 watch，加有界事件队列、debounce、目录范围重新核实和周期对账；watch 资源表与内核成本需要真实测量。fanotify 根据内核、权限与文件系统支持以后可选，不要求 root、capabilities 或 sysctl 改动。[官方监听语义与限制](linux-file-search-sources.md#5-linux-文件事件mvp-默认不需要提权)

| 情况 | 处理与恢复不变量 | 本轮证据 |
|---|---|---|
| 新增 | 安装目录 watch 再扫描新子树；验证记录加入 delta，重复事件幂等 | 小型真实夹具新增后重新扫描可见；无内核 watcher |
| 删除 | tombstone 遮盖旧 snapshot，打开动作重新验证存在性 | 记录模型与真实夹具删除后重新扫描验证 |
| 文件重命名 | cookie 有界配对；超时／移出范围按删除＋新增并局部核实 | 同 ID 路径更新模型、真实文件 rename 后重扫验证；未测 Linux cookie |
| 目录重命名／移入 | 更新目录节点，复核目录权限与 root 归属；补子树 watch | 设计项，未实现或计时 |
| 内核 IN_Q_OVERFLOW／用户队列丢失 | 标记 watcher 覆盖的 roots 为 Dirty；无法定位时重扫全部相关 roots；旧结果附 stale 状态 | 注入“丢事件”的状态模型经过对账恢复；未触发真实 overflow |
| 扫描与新事件竞态 | scan generation＋有界缓冲回放＋局部核实；缓冲溢出则重扫；旧扫描不得覆盖新事件 | 模型测试拒绝旧 generation 发布；尚无完整回放协议 |
| 重启／休眠／watch 注册失败／断挂载 | 标记 coverage 与 freshness；重新注册及对账；网络目录轮询降级 | 设计项，未实测 |
| 压缩合并／崩溃／损坏 | WAL／批次 generation、CRC、大小边界、写临时快照与原子 manifest 切换；仅成功快照可见 | 原型只测 save/load round-trip，**无 WAL、校验和、崩溃一致性保证** |

限制 delta 大小与合并频率，持续改名／大树删除不能永久增长内存。高事件率时丢弃细粒度队列并记 dirty，后台对账；对账持续失败保留错误与 stale 状态。至少一次事件不等于恰好一次应用，要通过 ID／路径复核去重。Linux stat 的 inode／device 可帮助关联，但需考虑 inode 复用、硬链接与 mount 身份，不把 inode 单独当作永远稳定 ID。

## 官方参考与复用判断

详细固定版本、许可证条款、组件形态与官方链接见 [来源核实](linux-file-search-sources.md)。本轮未引入任何外部 Rust 依赖。

- Cardinal MIT：`query-segmentation` 可独立评估；`search-cancel` 机制可借鉴但需实例级状态和更密取消检查；`namepool` 锁内逐项搜索且有 nightly 特性；`slab-mmap` 为临时映射容器，不能直接等同稳定持久格式。没有编译上游组件或完整依赖许可审计。
- plocate 官方 1.1.25：三字节 trigram／posting lists；无 trigram 分支扫描。核心与其 updatedb 改动为 GPL-2.0-or-later，继承 updatedb 声明 GPL 2。借鉴数据结构思想、自行实现，不复制源码。
- FSearch 0.3.2 已发布并有 inotify／fanotify 实现；主程序 GPL-2.0-or-later。学习事件丢失、重扫和 watch 故障场景，不复制或链接 GPL 代码。

## 分阶段推进与完成门槛

| 阶段 | 工作 | 可以结束这一阶段的证据 |
|---|---|---|
| 0：本轮 | 盘点 Kite、官方资料、Loci 独立 Rust 1.99 原型、Windows 合成记录基准与小夹具 | 可复现结果、已测／未测边界；没有正式功能修改 |
| 1：Loci 数据模型与查询 | 明确原始路径／身份／case／目录树语义；短词／扩展名／多词补强；比较 postings／块大小／压缩 | 10万／100万多语料与长路径压力，oracle／TopK正确性、取消、内存和峰值结果；不以单一重复语料达标作结 |
| 2：Loci Linux 文件系统 | 真实有界扫描、inotify、snapshot＋delta／WAL、对账与崩溃恢复 | ext4／Btrfs至少一实机，权限、symlink、rename cookie、watch限额、overflow、进程被杀、坏快照、网络降级故障注入；CPU/内存覆盖索引与watcher |
| 3：Kite 平台解耦与 Rust 升级 | Kite 项目级固定1.99；Windows target依赖隔离；FileSearch抽取；Linux `.desktop`／启动／文件动作；Iced显式X11/Wayland＋tiny-skia | Windows既有测试／release与关键运行回归；Linux build与库测试；启动安全／坏入口隔离；CPU渲染特性与实际界面验证 |
| 4：Linux MVP集成 | Kite调用Loci；应用与文件结果混排、文件模式／过滤、进度、stale／错误状态；热键、singleton、退出、托盘降级、XDG数据与打包 | 文件搜索首次建库与warm-start都可用；明确X11/Wayland矩阵；取消旧查询、真实打开／定位、中文IME、HiDPI、休眠／重连、安装卸载验证 |
| 5：独立发行准备 | 库接口／格式版本、CLI、README／bench／CI、依赖审计、LICENSE／通知、发布策略 | 用户决定许可证、兼容矩阵和发布授权；性能声明有环境／语料／方法，随后才公开推送或发布 |

顺序可以并行安排开发人员，但文件搜索不能被移出 Linux MVP。没有给工期或性能承诺；每阶段以证据作为接入门槛。

## 独立开源的利弊与待决定项

收益：查询／存储／监听可以独立基准和迭代，其他 Rust 应用可复用；Kite 不必携带引擎实现细节，性能与格式回归更清楚；Linux engine 问题和桌面问题能分开定位。代价：双项目版本、兼容与发布、文档／支持、跨仓库协作、依赖许可与接口维护；过早承诺稳定格式或多平台 watcher 会增加维护负担。

许可仅比较候选，未作决定：MIT 简洁宽松；MIT OR Apache-2.0 对 Rust 嵌入生态与专利条款较明确；MPL-2.0 可要求受覆盖文件修改的源码可用；GPL 家族将更显著影响组合发行和闭源调用方。官方条款与工程摘要已集中在 [许可候选表](linux-file-search-sources.md#9-独立开源-loci-的许可证候选待用户决定)。项目分拆或进程边界不自动消除上游义务。

需要用户之后决定：首发 Linux／桌面矩阵、推荐索引 roots 和隐藏／挂载策略、搜索语义与排序、是否期望 Windows 也使用 Loci、探索目标是否升级为验收指标及其统计口径、库接口稳定程度、开源许可证与正式发布时机。当前已有授权足以做原型验证，这些不作为本轮的阻塞或要求用户重复批准。

## 原型与结果入口

主原型／完整结果：`D:\Project\Loci`。方法、原始数据与 Windows 实测汇总见 [Loci 基准报告](loci-benchmark-results.md)。初始源码留在 `experiments/loci`；Kite 的 src、Cargo.toml／Cargo.lock、资源、安装器与正式 ADR 均未改变。
