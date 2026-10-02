# Linux 文件搜索与系统接口：官方来源核实

调查日期：2026-10-02。独立引擎名称为 **Loci**，Kite 是调用方。本文件仅提供方案研究与来源核实，未为 Loci 选定许可证，也未发布或推送任何仓库。本轮在 Windows 阅读公开资料及源码，**没有运行 Linux、Cardinal、plocate 或 FSearch**；性能不能引用成 Loci 或 Kite 的 Linux 实测。

依据仓库 [research skill](../../.agents/skills/research/SKILL.md) 以官方源码、发布包、协议及 Linux man-pages 为主。文中“事实”表示本轮读到的具体实现或规范；“建议／推断”是面向 Loci 的设计判断。仓库当前 [ADR 0002](../adr/0002-everything-file-search.md) 仍规定 Windows 文件搜索使用 Everything，本研究不修改该决策或正式功能。

用户后续明确 Loci 使用 **Rust 1.99**，独立验证工程放在 `D:\Project\Loci`，`Kite/experiments/loci` 保留早期实验快照。Rust 官方 release notes 确认 **1.99.0 于 2026-10-01 发布**。[官方发布说明](https://doc.rust-lang.org/releases.html#version-1990-2026-10-01) 验证时应显式选择 `cargo +1.99.0` 并记录 `rustc +1.99.0 -Vv`；研究代理本轮在 Kite 目录裸 `rustc --version` 仍得到 1.97.1，因此不能仅依据默认 PATH 宣称已用 1.99 构建。
## 1. 可复现的源码范围

| 项目 | 本轮固定范围 | 官方来源 |
|---|---|---|
| Cardinal | commit `4c50734f9a09d88110f96652b43634b412f79449`，提交时间 2026-06-26 | [固定提交](https://github.com/cardisoft/cardinal/commit/4c50734f9a09d88110f96652b43634b412f79449)、[commit API](https://api.github.com/repos/cardisoft/cardinal/commits/4c50734f9a09d88110f96652b43634b412f79449) |
| plocate | 官方发布包 **1.1.25**，官网标示 2026-09-06 发布 | [官网](https://plocate.sesse.net/)、[固定发布包](https://plocate.sesse.net/download/plocate-1.1.25.tar.gz) |
| FSearch | master commit `d531eb3b50560fb7d9ba731787100d827f4e1e8a`，提交时间 2026-09-22；另外核对发行版 **0.3.2** | [master 固定提交](https://github.com/cboxdoerfer/fsearch/commit/d531eb3b50560fb7d9ba731787100d827f4e1e8a)、[0.3.2 发布页](https://github.com/cboxdoerfer/fsearch/releases/tag/0.3.2) |
| FSearch 0.3.2 | commit `6d119c034410516cfb2104108fc96d1c29a7b222`，2026-09-20 | [固定发行提交](https://github.com/cboxdoerfer/fsearch/commit/6d119c034410516cfb2104108fc96d1c29a7b222)、[commit API](https://api.github.com/repos/cboxdoerfer/fsearch/commits/0.3.2) |
| Wayland 协议原文 | 官方历史发布包 **wayland-protocols 1.23** 内 `stable/xdg-shell/xdg-shell.xml`、`staging/xdg-activation/xdg-activation-v1.xml` | [官方发布列表](https://wayland.freedesktop.org/releases.html)、[1.23 发布包](https://wayland.freedesktop.org/releases/wayland-protocols-1.23.tar.xz) |

发布包 SHA-256（本轮从官方 URL 获取后在内存计算，供重复核对；未验证发布签名）：

- plocate 1.1.25：`68c1d5fbb11864403ae39c1a5937f13afd80b03e68041368831ef58cb613578e`
- wayland-protocols 1.23：`6c0af1915f96f615927a6270d025bd973ff1c58e521e4ca1fc9abfc914633f76`

当前 Wayland GitLab 原文请求返回反机器人页面，因此激活协议采用官网链接的历史发布包核实基础语义。它**不证明当前各桌面完整实现了这些接口**；portal 当前公开文档另行核对。

## 2. Cardinal：组件可复用性比整套移植更可靠

**事实：**仓库根 [LICENSE](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/LICENSE) 是 MIT，要求保留版权与许可通知。四个目标 crate 的 manifest 都声明 Rust **edition 2024**，不是 Kite 的 edition 2021；仓库 [rust-toolchain.toml](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/rust-toolchain.toml) 固定 `nightly-2025-12-11`。edition 2024 本身可在 Rust 1.85 起使用 stable，不意味着每个上游 crate 都只能 nightly；具体特性仍须逐 crate 检查。[Rust 官方 Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)

| 组件 | 已核实源码与依赖 | 对 Loci 的判断（建议／推断） |
|---|---|---|
| `query-segmentation` | [manifest](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/query-segmentation/Cargo.toml) 无外部依赖；[实现](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/query-segmentation/src/lib.rs) 按 `/` 切片，区分 substring、prefix、suffix、exact、`*`、`**`，有中文片段测试；没有此文件级平台绑定 | 最适合单独评估复用；它只是路径查询分段，不包含索引、召回、排序或完整 Everything 语法。需要先决定 Loci 路径语义，以及 Windows `\` 和连续 `/` 的处理。 |
| `search-cancel` | [manifest](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/search-cancel/Cargo.toml) 无外部依赖；[实现](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/search-cancel/src/lib.rs) 用两个全局 `AtomicU64` 维护 search／scan generation，新查询使旧 token 失效；稀疏检查间隔为 **65,536 次** | 可借鉴 generation 取消机制；多实例库应把状态归属到每个引擎或查询上下文，全局状态会造成实例互相取消。间隔是吞吐／取消延迟折中，不能当作毫秒级保证；长解压、交集、排序都需要检查点。 |
| `namepool` | [manifest](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/namepool/Cargo.toml) 依赖 memchr、rayon、serde、parking_lot、rustc-hash、regex、path `search-cancel`；[实现](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/namepool/src/lib.rs) 是 `Mutex<BTreeSet<Box<str>>>`，插入去重，substring／prefix／suffix／exact／regex 搜索都循环遍历集合；使用 `#![feature(str_from_raw_parts)]` 和 unsafe | 不推荐直接作为百万记录低内存引擎底座。它不是紧凑连续字符串池或 trigram 倒排索引；逐名称堆分配、锁内遍历与全量结果集合都需评估。现有源码存在 nightly 阻塞，注释中的 cache line／offset 描述不能替代实际结构。 |
| `slab-mmap` | [manifest](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/slab-mmap/Cargo.toml) 依赖 memmap2 0.9、tempfile 3.16、serde；[主体](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/slab-mmap/src/lib.rs) 将通用 `Entry<T>` 数组写入临时可写映射，容量翻倍时 flush／扩文件／remap；[serde](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/slab-mmap/src/serde.rs) 序列化为 key/value map，再重建 slab | 可作为存储技巧参考或独立试验对象；不能把临时映射原始 Rust 布局当作可跨版本重开的持久格式。`T` 内的 String／Vec 指向的堆内存不会自动搬入 mmap。文件格式、损坏校验、崩溃一致性与 unsafe 审核仍需 Loci 自己设计。 |

这些判断来自源码阅读，**本轮没有下载 crate、编译四组件、测试 Linux/Windows mmap 行为，或完整审计全部传递依赖许可证**。复用前必须固定 revision、带 MIT 通知、补 stable 编译与平台测试；这份研究不批准直接引入依赖。

**事实：**整套 SDK 不适合直接接入 Linux。固定 [cardinal-sdk manifest](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/cardinal-sdk/Cargo.toml) 描述 macOS 工具，直接依赖 CoreFoundation、CoreServices 的 FSEvents、dispatch2；[lsf manifest](https://github.com/cardisoft/cardinal/blob/4c50734f9a09d88110f96652b43634b412f79449/lsf/Cargo.toml) 又依赖 cardinal-sdk。这比 README 的“Rust 项目”描述更能解释平台边界。

## 3. plocate：磁盘索引与许可边界

**事实：**官网明确采用 **三字节** trigram 倒排索引与 posting lists 缩小候选；支持可选 io_uring 异步 I/O，最终结果还考虑目录权限可见性。官网的“2,700 万条、几毫秒”来自作者机器及特定稀少匹配示例，不能代替 Kite/Loci 基准。[plocate 官网](https://plocate.sesse.net/)

**事实：**读取官方 1.1.25 发布包可核实：

- `plocate.cpp` 的 `trigrams_to_lookup.empty()` 分支调用 `scan_all_docids`，无可提取 trigram 时退化到暴力扫描；短 ASCII 词或某些通配／正则表达式因此不是倒排优势场景。
- posting list 相交后还有最终文件名匹配校验；trigram 命中并不等于连续子串命中。
- `README` 指定 **plocate（不含继承的 updatedb）以及 plocate 对 updatedb 的改动：GPL 2 或任意更新版本**；继承的 updatedb 标明 **GPL 2**，未声明 later。`COPYING` 是 GPL version 2 文本。不能从 `COPYING` 单独推断每个文件是否带 later 选项。

以上来自 [官方 1.1.25 发布包](https://plocate.sesse.net/download/plocate-1.1.25.tar.gz) 中 `README`、`COPYING`、`plocate.cpp`，不是第三方许可证数据库。官方 git 页面注明需要 IPv6；本轮未以 git commit 固定发布包来源，已记录版本与 SHA-256。

**建议／推断：**Loci 借鉴“紧凑磁盘快照＋trigram 候选＋最终字节校验”，自行编写实现。为短词、扩展名、目录范围与大量匹配设置专门路径；不默认用全量结果排序。字节 trigram 与字符 trigram 不同：单个常见汉字的 UTF-8 字节长度可达到 3，不能把“少于 3 个字符”直接等同“无 trigram”。若要大小写折叠、中文／组合字符匹配，应固定原始路径与查询规范化规则，再验证假阳性校验和召回完整性。

本轮没有复用或复制 GPL 源码。通过独立进程调用既有工具与链接／复制代码是不同的技术边界，但这本身不能代替发行许可判断；Loci 的 MVP 也不应把 plocate 安装或系统数据库当成必需依赖。

## 4. FSearch：0.3 监听器已经进入发行版

**事实：**FSearch 由 C／GTK3 构建。根 `LICENSE` 是 GPL v2 文本，而主程序源头明确允许 **GPL 2 或更新版本**；因此仅引用 GitHub 的“GPL-2.0”徽章会遗漏 later 选项。[固定 LICENSE](https://github.com/cboxdoerfer/fsearch/blob/d531eb3b50560fb7d9ba731787100d827f4e1e8a/LICENSE)、[固定 src/fsearch.c](https://github.com/cboxdoerfer/fsearch/blob/d531eb3b50560fb7d9ba731787100d827f4e1e8a/src/fsearch.c)

**事实：**截至调查日，官方 latest release 是 **0.3.2（2026-09-20）**，发布说明包含数据库更新性能改善、未知 watch 事件崩溃修复及目录删除相关修复。0.3 文件系统监听不是单纯路线图。[0.3.2 发布说明](https://github.com/cboxdoerfer/fsearch/releases/tag/0.3.2)

源码事实：

- [发行版 fanotify 监听器](https://github.com/cboxdoerfer/fsearch/blob/6d119c034410516cfb2104108fc96d1c29a7b222/src/fsearch_folder_monitor_fanotify.c) 初始化 `FAN_CLASS_NOTIF | FAN_REPORT_DFID_NAME`，按目录 inode 添加 marks，维护文件句柄与目录的映射，并把存在多种 create/delete/move 位、顺序无法确定的事件转为 rescan。
- [发行版 inotify 监听器](https://github.com/cboxdoerfer/fsearch/blob/6d119c034410516cfb2104108fc96d1c29a7b222/src/fsearch_folder_monitor_inotify.c) 添加目录 watch，管理 watch descriptor 到目录映射，将 create/delete/move 等事件放入异步队列。
- [master meson.build](https://github.com/cboxdoerfer/fsearch/blob/d531eb3b50560fb7d9ba731787100d827f4e1e8a/meson.build) 编译期运行小程序检查无 `CAP_SYS_ADMIN` 的 fanotify 与 inotify 能力；[src/meson.build](https://github.com/cboxdoerfer/fsearch/blob/d531eb3b50560fb7d9ba731787100d827f4e1e8a/src/meson.build) 按结果加入相应监听器。

**建议／推断：**可学习“后台收事件、应用线程更新索引、失败回到重扫”的分层，不复制 C／GPL 代码。发布说明中的故障修复提醒我们必须验证 watch 注册失败、扫描取消后的迟到事件、目录移动／删除和事件丢失，而不只是能收到新增通知。静态源码存在并不证明所有发行版、挂载类型、权限与打包方式都能使用 fanotify。

## 5. Linux 文件事件：MVP 默认不需要提权

**事实：**inotify 目录监听不递归；新增／移入目录需注册子目录 watch，注册后立即扫描其内容以缩小竞态窗口。rename 的 `IN_MOVED_FROM`／`IN_MOVED_TO` 通过 cookie 关联，但不保证相邻或同时入队，移出监控范围还可能没有 TO。`IN_Q_OVERFLOW` 的 wd 为 -1，发生时事件已丢失；内核限制包括用户 watch 数及队列长度。网络文件系统远端变化还可能需要轮询。[Linux man-pages inotify(7)](https://man7.org/linux/man-pages/man7/inotify.7.html)

**事实：**fanotify 非特权调用从 Linux 5.13 起允许创建受限 group，官方文档还列出 5.10.220 回移植例外。无 `CAP_SYS_ADMIN` 时不能要求无限队列／marks，不能使用 permission event classes，需通过文件句柄标识对象，只允许 inode marks，不能 mark 整个 mount／filesystem；内核配置也必须启用 fanotify。[fanotify_init(2)](https://man7.org/linux/man-pages/man2/fanotify_init.2.html) 某些 flags、文件系统文件句柄支持及路径权限仍会使注册失败。[fanotify_mark(2)](https://man7.org/linux/man-pages/man2/fanotify_mark.2.html)

**建议／推断：**Loci 首版用普通用户的 inotify＋限定目录树＋周期一致性对账。fanotify 作为以后按运行时能力选择的优化，不要求用户更改 sysctl、授予 capabilities 或启动 root daemon。每目录 watch 的用户空间表与内核资源都要计入运维和内存评估。

恢复设计建议：

1. 快照扫描前为根／目录逐步建立监听，记录 scan generation 与有界事件缓冲；扫描后回放事件，必要时局部对账。新目录先建 watch 再扫描子树。
2. rename cookie 暂存采用超时与容量上限；配对失败按删除／新增处理，并对受影响目录复核；目录重命名以 parent ID／name 更新减少全树字符串重写。
3. 先识别 overflow 等特殊事件，再查 watch descriptor 映射，不能把 wd=-1 当成未知目录断言失败。
4. overflow、应用队列丢事件、重启／断线、watch 注册失败、目录失联时将相关 root 标为 stale／dirty；不能根据剩余事件宣称索引完整。事件无法定位丢失范围时重扫该 watcher 覆盖的全部 roots。
5. 后台重建新 generation，完成校验后原子切换；保留可查询旧快照但显式报告 freshness，定期对账修复静默遗漏。网络挂载或无法监听目录降级到有界轮询。

这些是 Loci 设计建议，**没有在本轮 Windows 原型里证明 Linux 监听可靠性或恢复正确性**。

## 6. X11 与 Wayland：不能统一承诺窗口控制

**事实：**Kite 当前 [Cargo.toml](../../Cargo.toml) 对 Iced 关闭 default features，启用 tiny-skia、crisp、thread-pool、image、canvas、unconditional-rendering，但没有 x11／wayland。Iced 官方 0.14.0 manifest 将 tiny-skia 明确列为软件渲染器，把 x11／wayland 各自传递给 iced_renderer 与 iced_winit；默认特性还包含 wgpu，因此不能为了 Linux 后端简单恢复全部 defaults。[Iced 0.14.0 官方 Cargo.toml](https://github.com/iced-rs/iced/blob/0.14.0/Cargo.toml)

**建议：**Linux 条件依赖显式启用 x11 与 wayland，保留 tiny-skia 的 CPU 渲染；不启用 wgpu。编译特性正确仅解决后端构建前提，仍需验证两个会话实际开窗、输入法、缩放与 CPU／内存。这里仅核实特性与本地现状，没有修改正式 Cargo.toml。
| 能力 | 官方事实 | Kite 适配建议／待实测 |
|---|---|---|
| X11 全局热键 | XGrabKey 提供 passive grab；同一键与修饰组合已被另一客户端 grab 时发生 BadAccess。[X.Org XGrabKey](https://xorg.freedesktop.org/archive/X11R7.5/doc/man/man3/XGrabKey.3.html) | 基于 root window grab 注册，处理占用、键盘布局／修饰状态与重注册。X11 会话需实测，不将 XWayland grab 视为原生 Wayland 全局热键保障。 |
| X11 激活 | EWMH `_NET_ACTIVE_WINDOW` 是给窗口管理器的请求，带来源与用户活动 timestamp；窗口管理器可以拒绝。[EWMH 1.5 §3.8](https://specifications.freedesktop.org/wm/1.5/ar01s03.html) | 通过用户操作上下文请求激活；实测目标 WM 的 focus stealing 策略，保留正常启动窗口入口。 |
| Wayland 全局热键 | XDG GlobalShortcuts portal 允许建立 session、BindShortcuts、接收 Activated；绑定通常弹出用户配置对话框，实际绑定结果可以是空集。Activated options 可带窗口 `activation_token`。本轮文档描述 interface v2。[官方 portal 文档](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) | 运行时探测接口版本、session／绑定结果及当前 backend。没有服务、用户拒绝或未绑定时，提供桌面快捷键执行 `kite --toggle` 的配置说明及 `.desktop` 常规入口；不自动改系统设置。 |
| Wayland 激活 | 官方 1.23 `xdg-activation-v1.xml` 中 token 可以无效，合成器决定是否响应 activate，可以忽略未知 token。[官方协议发布包](https://wayland.freedesktop.org/releases/wayland-protocols-1.23.tar.xz) | 将 portal／启动参数中的有效 token 传递到 Iced／winit 的激活路径，并验证其实际支持；不能保证后台任意时刻抢焦点。 |
| Wayland 窗口定位 | Wayland 官方说明客户端不知道 surface 全局位置，也不能访问其他客户端 surfaces。[官方 Protocol 文档](https://wayland.freedesktop.org/docs/book/Protocol.html#surfaces) 1.23 xdg-shell toplevel 提供的是用户驱动 move，无任意全局坐标 set-position 请求。[官方协议发布包](https://wayland.freedesktop.org/releases/wayland-protocols-1.23.tar.xz) | 普通窗口接受合成器布局；“鼠标所在屏精准居中”不能作为通用 Wayland 承诺。layer-shell／桌面插件属于单独适配面，不列为全部桌面 MVP 必需前提。 |

这里核实的是协议语义，**没有核实 GNOME／KDE／wlroots 当前每个版本的 portal 后端矩阵，也未测试 Iced 0.14／项目锁定 winit 的 token 接口**。Linux 验收至少覆盖一个 X11 会话、GNOME Wayland、KDE Wayland；wlroots 应明确支持的合成器及 portal 后端组合。窗口显隐、输入法、焦点恢复、多屏缩放和休眠重连都必须实机验证。

## 7. Linux 应用入口不是 shell 字符串

**事实：**freedesktop Desktop Entry 的 `Exec` 有自己的参数引用、转义和 field codes 语法；未知 field code 使命令无效，替换文件路径时必须保留参数边界。`Exec` 字符串不能当成随意执行的 shell 输入。[Desktop Entry：Exec](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html)

**事实：**索引／展示／启动还应处理 `Type=Application`、本地化 Name、Keywords、Icon、Hidden、NoDisplay、OnlyShowIn／NotShowIn、TryExec、Path、Terminal。`DBusActivatable=true` 时标准要求支持者通过 D-Bus 激活，不优先执行 Exec。[Desktop Entry：标准键](https://specifications.freedesktop.org/desktop-entry/latest/recognized-keys.html)

**建议／推断：**Kite Linux 使用独立 `.desktop` parser 与已验证 `LaunchTarget`，按参数向量执行，或使用符合 Desktop Entry 的系统 API。搜索输入只负责筛选索引项，不拼到 `/bin/sh -c`。Loci 只返回文件记录／标识／路径及 freshness，不拥有应用启动权限或命令解析职责。

## 8. mmap 与内存口径

**事实：**Linux `/proc/pid/status` 的 VmRSS 是 RssAnon、RssFile 与 RssShmem 之和，RssFile 包括驻留文件映射，计数具有采样精度限制。[proc_pid_status(5)](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html) `/proc/pid/smaps` 可按映射观察 Size、Rss、Pss、私有／共享脏页等。[proc_pid_smaps(5)](https://man7.org/linux/man-pages/man5/proc_pid_smaps.5.html)

**建议／推断：**mmap 减少手动全量装载不等于零内存；被访问的索引页、页表、解压缓冲、候选／排序堆、delta、watch 表都有成本。Loci 基准分别报告磁盘索引、虚拟映射、RSS／PSS、私有内存、峰值及加载／首查／热查后的状态；不能用较小 private bytes 隐去映射驻留页。“新进程首查”也不等于 OS page cache 真冷；不更改系统设置时应标为“进程冷、缓存未清”。

## 9. 独立开源 Loci 的许可证候选（待用户决定）

以下仅是公开条款的工程摘要，不在本轮写入 LICENSE 或 Cargo license，不承诺法律兼容结论。

| 候选 | 主要取向与工程代价 | 官方条款 |
|---|---|---|
| MIT | 简单宽松、易被其他应用采用；需要保留版权／许可通知，没有 Apache 式单列专利条款 | [OSI MIT 正文](https://opensource.org/license/mit) |
| MIT OR Apache-2.0 | 调用方可选择许可；Apache 2.0 提供明确贡献者专利授权及终止条件、通知等要求，维护两套许可与依赖清单 | [Apache 2.0 正文](https://www.apache.org/licenses/LICENSE-2.0)、[MIT 正文](https://opensource.org/license/mit) |
| MPL-2.0 | 希望修改到受覆盖文件的代码在发行时继续公开，允许符合条款的 Larger Work 使用其他许可；文件边界和 source availability 增加管理工作 | [Mozilla MPL 2.0 正文](https://www.mozilla.org/en-US/MPL/2.0/) |
| GPL 系列 | 选择更强的源码公开／衍生发行条件；会显著影响潜在闭源调用方和 Kite 的组合发行判断；不能默认与所有宽松许可证版本兼容 | [FSearch 固定 GPL v2 文本](https://github.com/cboxdoerfer/fsearch/blob/d531eb3b50560fb7d9ba731787100d827f4e1e8a/LICENSE)、[plocate 官方许可声明所在发布包](https://plocate.sesse.net/download/plocate-1.1.25.tar.gz) |

**建议：**如果优先让 Loci 成为可嵌入的 Rust 库，可先比较 MIT 与 MIT OR Apache-2.0；如果优先保证引擎文件改动回馈社区，再讨论 MPL-2.0。项目拆分或进程边界不能自动解决上游代码许可义务。无论选择哪种，正式发布前核查作者／贡献者权属、拟复用的每个文件、传递依赖、通知与商标；本次只研究，不复制 GPL 实现。

## 10. 研究边界与下一轮验证

本轮完成官方许可证版本、组件源码形态、发行进度与系统接口语义核实；没有将任何第三方代码接入 Kite，也没有编译／运行第三方程序。

下一轮需要实际验证：Cardinal 小组件的 stable toolchain 最低版本与跨平台测试、完整依赖许可证；Linux 文件系统和 watcher 的权限／限额／overflow／rename／挂载恢复；目标桌面的 portal 绑定和 activation token；项目 Iced/winit 的后端行为；真实 Linux 内存与冷热 I/O。当前 Windows 合成记录基准只能解释 Loci 原型的数据结构与查询开销，不能证明 Linux MVP 已完成或 Linux 上满足 100 ms／100 MB 探索目标。
