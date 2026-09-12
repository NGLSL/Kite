# Kite 产品需求文档

## 1. 项目概述

### 1.1 项目名称

**Kite**

### 1.2 产品定位

Kite 是一个面向 Windows 的轻量桌面启动器。

核心目标：

> **获得接近或超过 uTools 的搜索体验，同时保持接近 Flow Launcher 的轻量和低资源占用。**

Kite 不追求成为功能最多的桌面工具箱。

优先解决的问题是：

> 用户知道自己大概想找什么，但不一定能准确输入完整名称时，Kite 依然能够快速、稳定地把正确结果排在最前面。

---

# 2. 项目背景

目前常见 Launcher 存在两个方向的问题。

### uTools

优点：

- 搜索体验较好
- 中文搜索体验较好
- 软件、插件、工具整合度高

问题：

- 常驻资源占用较高
- 整体功能较重

### Flow Launcher

优点：

- 相对轻量
- Windows Launcher 功能完整
- 插件生态成熟

问题：

- 某些搜索场景下结果不够符合预期
- 中文、简称、模糊输入、用户习惯排序等体验不如预期

Kite 希望在两者之间找到一个新的平衡点：

> **轻量，但搜索足够聪明。**

---

# 3. 核心产品原则

Kite 的功能开发按照以下优先级排序：

1. 搜索准确度
2. 搜索响应速度
3. 窗口唤起速度
4. 内存和 CPU 占用
5. 键盘操作体验
6. UI 体验
7. 扩展能力
8. 插件生态

新增任何功能前需要判断：

- 是否改善搜索体验？
- 是否明显增加后台资源占用？
- 是否属于当前阶段真正需要的能力？

如果答案都是否，则暂缓实现。

---

# 4. 技术栈

当前项目已经完成初始化。

技术栈：

```text
Rust
Tauri 2
React
TypeScript
Vite
```

运行平台：

```text
Windows 11 x64
```

Rust 工具链：

```text
stable-x86_64-pc-windows-msvc
```

后续需要持久化数据时考虑加入：

```text
SQLite
```

---

# 5. 技术职责边界

## 5.1 Rust

Rust 负责核心业务能力：

```text
应用扫描
应用索引
Query 规范化
搜索召回
拼音处理
Alias
模糊匹配
评分
Ranking
用户历史
应用启动
Windows 系统集成
```

## 5.2 React

React 主要负责：

```text
搜索输入框
搜索结果展示
键盘交互
设置界面
简单状态展示
```

不允许将核心搜索算法实现到 React / TypeScript 中。

整体保持：

```text
React
   ↓
Tauri IPC
   ↓
Rust
   ↓
Search Engine
```

---

# 6. 当前明确不做的内容

第一阶段以及搜索核心稳定之前，不考虑：

```text
插件市场
完整第三方插件系统
AI Assistant
OCR
截图
云同步
账户系统
跨设备同步
Linux
macOS
复杂主题系统
工作流编排
向量数据库
Embedding 搜索
机器学习 Ranking
自建文件全文索引
```

不要因为未来可能需要这些功能而提前设计复杂抽象。

---

# 7. 产品发展阶段

Kite 按阶段开发。

---

# Phase 1：基础 Launcher

## 8. Phase 1 目标

Phase 1 的目标不是实现完整搜索能力。

而是建立：

> **一个真正可以使用的最小 Windows Launcher。**

完成之后用户应该能够：

```text
Alt + Space
↓
输入应用名称
↓
找到应用
↓
Enter
↓
启动
```

---

# 9. 全局快捷键

默认快捷键：

```text
Alt + Space
```

行为：

### Kite 隐藏时

按：

```text
Alt + Space
```

Kite：

- 显示搜索窗口
- 搜索框获得焦点
- 可以立即输入

### Kite 显示时

再次按快捷键：

```text
Alt + Space
```

可以隐藏 Kite。

---

# 10. 窗口行为

Kite 主窗口应：

- 无明显启动延迟
- 默认显示在当前主要操作屏幕
- 搜索输入框自动获得焦点
- 不显示不必要的窗口边框
- 不应该抢占系统资源

支持：

```text
Esc
```

隐藏窗口。

启动应用后自动隐藏窗口。

窗口隐藏后：

- 不持续进行 UI 重绘
- 不持续高频轮询
- CPU 应接近 0%

---

# 11. 中文输入法

必须保证：

- 中文 IME 可以正常使用
- 输入候选状态不会因为搜索刷新而被打断
- 搜索结果刷新不能导致输入框失焦
- 快速连续输入时不能明显卡顿

---

# 12. Windows 应用扫描

Phase 1 只扫描应用入口。

不要扫描整个硬盘中的 `.exe`。

优先考虑：

```text
开始菜单
用户开始菜单
公共开始菜单
用户桌面
公共桌面
Windows Apps
常见 App Paths
```

需要识别：

```text
.lnk
.exe
UWP / packaged app
```

---

# 13. 应用统一模型

不同来源扫描出来的应用需要转换成统一结构。

概念模型：

```text
AppItem

id
name
display_name
target
icon
source
```

后续可以扩展：

```text
aliases
normalized_name
pinyin
pinyin_initials
```

但是 Phase 1 不需要提前实现所有字段。

---

# 14. 应用去重

同一个应用可能同时出现在：

```text
开始菜单
桌面
App Paths
```

Kite 只能显示一个。

需要建立稳定的去重规则。

优先使用：

```text
实际 target
应用唯一标识
规范化路径
```

进行判断。

不能简单只按名称去重，因为：

```text
Visual Studio Code
Visual Studio
```

不能被误认为同一个应用。

---

# 15. 图标

搜索结果需要显示：

```text
应用图标
应用名称
```

图标获取失败不能影响搜索。

获取失败时允许显示统一默认图标。

图标处理不能阻塞搜索线程。

后续可以增加缓存。

---

# 16. Phase 1 搜索能力

Phase 1 只需要：

```text
Exact
Prefix
```

例如：

```text
chrome
→ Google Chrome
```

以及：

```text
vis
→ Visual Studio Code
```

暂时不要实现：

```text
拼音
Alias
Fuzzy
History Ranking
```

---

# 17. 搜索结果数量

默认只返回：

```text
Top 10
```

未来可以设置。

Rust 不应该每次把完整应用列表发送到 React。

---

# 18. 键盘控制

必须支持完全键盘操作。

```text
↑
↓

切换搜索结果
```

```text
Enter

启动当前选择项
```

```text
Esc

关闭 Kite
```

输入 Query 后：

默认选择第一条结果。

---

# 19. Phase 1 验收标准

Phase 1 完成条件：

### 功能

能够：

```text
Alt + Space
↓
出现 Kite
↓
输入 Chrome
↓
出现 Chrome
↓
Enter
↓
启动 Chrome
↓
Kite 隐藏
```

必须支持：

- 应用扫描
- 应用去重
- 应用图标
- Exact
- Prefix
- 上下选择
- Enter
- Esc

---

# Phase 2：搜索核心

# 20. Phase 2 目标

Phase 2 是 Kite 最重要的阶段。

目标：

> **让 Kite 的搜索体验开始明显优于普通 fuzzy launcher。**

加入：

```text
Normalizer
Alias
中文
拼音
拼音首字母
Substring
Fuzzy
统一 Ranking
```

---

# 21. Search Engine 总体流程

搜索采用：

> **多路召回 + 统一排序**

流程：

```text
Query
 ↓
Normalizer
 ↓
Candidate Retrieval
 ├─ Exact
 ├─ Alias
 ├─ Prefix
 ├─ Pinyin
 ├─ Pinyin Initial
 ├─ Substring
 └─ Fuzzy
 ↓
Ranker
 ↓
Top N
```

不要设计成：

```text
Query
↓
一个 fuzzy 算法
↓
直接按照 fuzzy score 排序
```

---

# 22. Query Normalizer

输入：

```text
"  VS Code  "
```

处理为：

```text
vs code
```

需要处理：

```text
大小写
前后空格
连续空格
Unicode 基础规范化
常见符号
```

Normalizer 应独立于 Matcher。

---

# 23. Alias 系统

Alias 是 Kite 的核心能力之一。

系统内置部分高频 Alias。

例如：

```text
vs
vsc
vscode
→ Visual Studio Code
```

```text
wx
weixin
wechat
→ 微信
```

```text
qywx
wxwork
→ 企业微信
```

```text
idea
→ IntelliJ IDEA
```

内置 Alias 原则：

> 少而准。

不要维护上千个冷门短词。

---

# 24. 用户 Alias

后续支持用户配置：

```text
query → App
```

例如：

```text
code → Visual Studio Code
```

用户定义 Alias 优先级应最高。

---

# 25. 中文搜索

支持：

```text
微信
```

直接匹配：

```text
微信
```

汉字 Exact / Prefix 应属于高质量匹配。

---

# 26. 拼音搜索

索引阶段提前生成：

```text
微信

weixin
wx
```

用户可以输入：

```text
weixin
```

得到：

```text
微信
```

也可以：

```text
wx
```

得到：

```text
微信
```

例如：

```text
微信开发者工具

weixinkaifazhegongju
wxkfzgj
```

允许匹配：

```text
wxkf
```

---

# 27. 拼音预计算

不要每次搜索：

```text
遍历所有应用
↓
重新转换拼音
```

应用进入索引时提前生成：

```text
normalized_name
pinyin
pinyin_initials
```

搜索阶段只比较字符串。

---

# 28. Prefix

例如：

```text
vis
```

匹配：

```text
Visual Studio Code
```

Prefix 属于高质量匹配。

---

# 29. Substring

例如：

```text
studio
```

可以匹配：

```text
Visual Studio Code
Visual Studio
Android Studio
```

Substring 的 Ranking 应低于 Exact 和 Prefix。

---

# 30. Fuzzy Search

Fuzzy 主要解决：

> 用户记不清或者打错。

例如：

```text
chorme
→ Chrome
```

```text
crome
→ Chrome
```

```text
chrom
→ Chrome
```

可以考虑：

```text
Damerau-Levenshtein
```

因为需要良好处理：

```text
chorme
chrome
```

这种相邻字符交换。

也可以评估成熟 Rust fuzzy matcher。

---

# 31. Fuzzy 限制

Fuzzy 不应过于宽松。

建议：

```text
长度 <= 2
禁止大范围 fuzzy
```

```text
长度 3~5
编辑距离最大约 1
```

```text
长度 >= 6
编辑距离最大约 1~2
```

具体阈值需要通过测试调整。

---

# 32. 搜索质量优先级

初始逻辑优先级：

```text
User Alias Exact
↓
Name Exact
↓
Built-in Alias Exact
↓
Name Prefix
↓
Pinyin Exact
↓
Pinyin Initial Exact
↓
Substring
↓
Fuzzy
```

注意：

这只是行为优先级。

最终实现统一通过 Ranking Score 控制。

---

# 33. Ranking

建议统一计算：

```text
FinalScore =
    MatchScore
  + AliasScore
  + HistoryScore
```

Phase 2 暂时主要使用：

```text
MatchScore
AliasScore
```

Phase 3 再加入用户历史。

---

# 34. 初始分数模型

初始可以参考：

```text
User Alias Exact       1100

Name Exact             1000

Built-in Alias Exact    950

Prefix                  800

Pinyin Exact            750

Pinyin Initial          700

Substring               550

Fuzzy                  0~450
```

这只是初始配置。

分数不要散落在各 Matcher 内。

集中定义。

---

# Phase 3：个性化排序

# 35. Phase 3 目标

让 Kite：

> **越用越懂用户。**

加入：

```text
Query History
Frequency
Recency
SQLite
```

---

# 36. 不使用纯 LRU

Kite 不应该直接：

```text
最近使用
=
最高排名
```

因为 Launcher 搜索不是缓存淘汰。

需要综合：

```text
Frequency
+
Recency
+
Query History
```

---

# 37. Query History

这是个性化 Ranking 最重要的部分。

记录：

```text
query
→
用户最后选择哪个 App
```

例如：

```text
wx → 微信
```

用户连续 20 次：

```text
wx
```

都选择：

```text
微信
```

以后：

```text
wx
```

Kite 应该稳定把：

```text
微信
```

放第一。

---

# 38. Query History 数据

概念上记录：

```text
query
item_id
count
last_used_at
```

例如：

```text
wx → 微信 → 27 次

wx → 微信开发者工具 → 2 次
```

---

# 39. Frequency

记录：

```text
应用总启动次数
最近一段时间启动次数
```

可以考虑：

```text
7 天
30 天
```

长期 Frequency 应有一定衰减。

不能让两年前高频使用的软件永远拥有巨大权重。

---

# 40. Recency

记录：

```text
last_used_at
```

最近使用应用获得适量加分。

Recency 权重应受到限制。

不能出现：

> 刚刚偶尔打开一次的软件直接把长期常用软件顶掉。

---

# 41. 个性化 Ranking

Phase 3：

```text
FinalScore =
    MatchScore
  + AliasScore
  + QueryHistoryScore
  + FrequencyScore
  + RecencyScore
```

重要原则：

> **MatchScore 永远是主要信号。**

History 只能帮助解决模糊情况。

不能覆盖明确意图。

---

# 42. 明确匹配保护

例如用户输入：

```text
chrome
```

即使：

```text
Chrome Remote Desktop
```

最近使用很多次，

只要：

```text
Google Chrome
```

存在高质量 Exact 匹配，

Google Chrome 就不应该因为 History 被压下去。

---

# 43. 动态 Alias

后续可以把稳定的 Query History 视为：

```text
Learned Alias
```

例如：

```text
wx → 微信
```

长期成立。

不一定真的修改 Alias 配置。

可以内部提高 Query-App 关系权重。

---

# 44. SQLite

Phase 3 开始使用 SQLite。

主要持久化：

```text
应用信息
Alias
用户启动历史
Query History
配置
```

概念表：

```text
items
aliases
usage_history
query_history
settings
```

不要为了 SQLite 引入复杂重量级 ORM。

优先简单、明确的数据访问层。

---

# Phase 4：产品完善

# 45. 设置

第一批设置只需要：

```text
全局快捷键

开机启动

失焦是否隐藏

结果数量

自定义 Alias

重新扫描应用
```

不要做几十个配置项。

---

# 46. 托盘

支持系统托盘：

```text
打开 Kite

设置

重新扫描

退出
```

---

# 47. 开机启动

用户可以选择：

```text
随 Windows 启动
```

必须允许关闭。

---

# 48. 索引刷新

第一版：

```text
Kite 启动
↓
扫描
```

支持：

```text
手动重新扫描
```

后续再考虑监听 Windows 应用变化。

避免：

```text
每 N 秒扫描整个系统
```

---

# 49. 多显示器

需要逐步完善：

- 当前活动屏幕显示
- DPI 正确
- 分辨率变化正常
- 多屏移动稳定

---

# 50. UI

整体风格：

> 简单、紧凑、信息密度适中。

主窗口第一阶段：

```text
┌──────────────────────────────┐
│ 🔍 Search...                  │
├──────────────────────────────┤
│ Icon  Visual Studio Code      │
│ Icon  Visual Studio           │
│ Icon  VS Installer            │
└──────────────────────────────┘
```

不要：

- 巨型 Card
- 复杂动画
- 重度毛玻璃
- 大量装饰
- 搜索时出现布局跳动

---

# 51. 动画

动画只能服务体验。

允许：

```text
窗口淡入
选择项轻微过渡
```

但：

> 输入 → 搜索结果

不能等待动画。

---

# 52. 后续功能

搜索启动器稳定后再考虑：

```text
Everything 文件搜索
文件夹启动
URL
网页搜索
计算器
命令执行
系统设置
内置小工具
```

---

# 53. Everything

文件搜索不要自行重新实现 NTFS 全盘索引。

优先评估：

```text
Everything IPC / SDK
```

Kite 只负责：

```text
Query
↓
调用 Everything
↓
统一 Ranking / 展示
```

---

# 54. 插件系统

插件属于较后阶段。

只有满足：

```text
Launcher 稳定
Search 稳定
History Ranking 稳定
内存满意
CPU 满意
日常已经可以替代 Flow
```

之后才设计。

禁止现在为了未来插件：

```text
做 Plugin Runtime
做 SDK
做动态加载
拆大量 crates
```

---

# 55. Rust 模块建议

初期保持单 crate。

可以逐步整理：

```text
src-tauri/src/

app/
    scanner
    launcher

search/
    normalizer
    alias
    pinyin
    matcher
    fuzzy
    ranker

history/
    usage
    query

storage/
    sqlite

system/
    hotkey
    window
```

这里是职责建议，不要求项目初始化后立即创建所有目录。

实际需要时再拆。

---

# 56. 禁止过度工程

当前不要建立：

```text
kite-core
kite-runtime
kite-search
kite-platform
kite-plugin
kite-common
kite-infra
```

除非未来真的出现：

- 独立复用需求
- 编译边界需求
- 多前端需求
- 明显模块耦合问题

---

# 57. 性能要求

## 搜索性能

目标：

```text
P95 < 50 ms
```

理想：

```text
< 20 ms
```

场景：

```text
用户按键
↓
Rust Search
↓
Top N
↓
React Render
```

---

# 58. IPC

React 不要一次请求所有应用再自己过滤。

应该：

```text
search(query)
```

Rust 返回：

```text
Top N SearchResult
```

这样：

- IPC 数据更少
- 搜索逻辑统一
- 前端更简单

---

# 59. 内存目标

这是 Kite 核心指标之一。

测试必须统计：

> Kite 整个进程树。

包括：

```text
Kite Rust process
+
WebView2 related processes
```

不能只报告：

```text
kite.exe = xx MB
```

---

# 60. Idle 内存目标

第一阶段目标：

```text
尽量 < 100 MB
```

理想：

```text
60~80 MB 或更低
```

该目标不是绝对死线。

重点是：

> 明显低于重型 Electron 工具，并且长期运行稳定。

---

# 61. CPU 目标

Kite 隐藏空闲时：

```text
CPU ≈ 0%
```

不应该存在持续：

```text
轮询
重新索引
UI 更新
计时任务
无意义 IPC
```

---

# 62. 内存稳定性

测试：

```text
打开 Kite
关闭
打开
关闭
```

重复：

```text
100 次
```

观察内存。

不应该持续无上限增长。

---

# 63. Search Regression Suite

搜索必须建立回归测试。

基础数据：

```text
Query        Expected

wx           微信

wxkf         微信开发者工具

weixin       微信

vsc          Visual Studio Code

vscode       Visual Studio Code

idea         IntelliJ IDEA

chorme       Google Chrome

crome        Google Chrome
```

---

# 64. 测试原则

以后任何搜索 Bug：

例如：

> 输入 `xxx`，正确程序应该第一，但 Kite 排第三。

修复以后：

> 将该 Query 加入测试。

长期积累：

```text
Kite Search Regression Suite
```

---

# 65. Ranking 回归

修改 Ranking 时：

必须确保：

> 修复一个搜索场景不能导致另外十个场景退化。

因此 Ranking 相关代码应该具有较完整单元测试。

---

# 66. Debug 信息

开发模式下建议允许输出：

```text
Query

Candidate

Matched By

Base Score

History Score

Final Score
```

例如：

```text
Query: wx

微信
AliasExact: 950
History: +180
Final: 1130

微信开发者工具
PinyinPrefix: 700
History: +20
Final: 720
```

方便定位：

> 为什么它会排第一？

Release 默认不显示这些信息。

---

# 67. 日志

只记录必要信息：

```text
启动
索引
错误
严重异常
```

搜索每次击键不要默认大量写磁盘日志。

否则会造成：

- IO
- 磁盘写入
- 性能下降

---

# 68. 错误处理

单个应用：

```text
图标失败
快捷方式损坏
target 不存在
```

不能导致：

```text
整个索引失败
```

应该：

```text
跳过 / 降级
```

并保留适当 Debug 信息。

---

# 69. 安全

启动应用时：

只执行索引中明确记录的 target。

不要：

```text
把用户搜索字符串直接拼接到 shell command
```

Shell / command 功能未来独立设计。

---

# 70. 开发原则

Agent 开发过程中：

### 简单任务

直接实现。

### 需求不明确

使用：

```text
grilling
```

### 涉及已有文档和复杂规则

使用：

```text
grill-with-docs
```

### 模块职责复杂

需要时：

```text
domain-modeling
codebase-design
```

### 难排 Bug

使用：

```text
diagnosing-bugs
```

### 较大的跨会话需求

讨论清楚后才使用：

```text
to-spec
```

### 功能完成

根据改动大小使用：

```text
code-review
```

不要每个任务自动执行全部 Skills。

---

# 71. Agent 实施规则

默认遵循：

> 最小充分方案。

禁止：

- 提前设计未来功能
- 为了理论优雅增加多层抽象
- 没有需求就新增 dependency
- 简单功能建立复杂框架
- 未验证问题就猜测式修改
- 一次实现多个 Phase

---

# 72. Phase 1 开发顺序

推荐：

## Task 1

窗口行为：

```text
Alt + Space
Esc
Show
Hide
Focus
```

## Task 2

Windows App Scanner：

```text
扫描
解析
去重
```

## Task 3

Result UI：

```text
Icon
Name
Selection
```

## Task 4

Search v1：

```text
Exact
Prefix
```

## Task 5

Launch：

```text
Enter
启动
隐藏 Kite
```

## Task 6

Phase 1 性能检查。

---

# 73. Phase 2 开发顺序

推荐：

```text
Normalizer
↓
Alias
↓
Pinyin
↓
Initial
↓
Substring
↓
Fuzzy
↓
Unified Ranker
↓
Regression Tests
```

不要一次全部实现。

---

# 74. Phase 3 开发顺序

```text
SQLite
↓
Usage History
↓
Frequency
↓
Recency
↓
Query History
↓
History Ranking
↓
Ranking 回归测试
```

---

# 75. Kite v1.0 定义

Kite 达到以下状态，可以认为进入 v1：

### Launcher

- 稳定唤起
- 稳定隐藏
- Windows 应用扫描稳定
- 应用启动稳定

### Search

支持：

```text
中文
英文
Exact
Prefix
Substring
拼音
拼音首字母
Alias
Fuzzy
```

### Personalized Ranking

支持：

```text
Query History
Frequency
Recency
```

### 产品

支持：

```text
自定义快捷键
开机启动
自定义 Alias
索引刷新
基础设置
```

### 性能

- 搜索无明显延迟
- 空闲 CPU 接近 0%
- 内存占用符合轻量定位
- 长时间运行稳定

---

# 76. Kite 暂时不以这些指标为目标

Kite v1 不要求：

```text
插件数量
功能数量
AI 能力
跨平台
云同步
团队协作
```

评价 Kite 成功与否主要看：

```text
Top 1 搜索正确率
Top 3 搜索正确率
输入到结果延迟
窗口唤起速度
Idle CPU
Idle Memory
日常替代 Flow Launcher 的可用性
```

---

# 77. 最终愿景

Kite 的核心体验应该是：

用户按：

```text
Alt + Space
```

输入：

```text
wx
```

Kite 知道：

> 用户大概率是想打开微信。

输入：

```text
wxkf
```

Kite 又知道：

> 这次用户想找微信开发者工具。

输入：

```text
chorme
```

即使拼错：

> Kite 仍然能够找到 Chrome。

随着使用：

> Kite 越来越符合用户自己的输入方式。

最终产品定位：

> **轻、快、搜得准，而且越用越懂用户。**