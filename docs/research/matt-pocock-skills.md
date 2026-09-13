# Matt Pocock skills 上游目录调查

调查日期：2026-09-13

本报告只使用 Matt Pocock 的官方仓库作为来源：[mattpocock/skills](https://github.com/mattpocock/skills)。目录和文件按 `main` 当前可见版本核对，报告采用的可复现提交是 [`3cca18b368ae95cdbdebbff572ccafa662551015`](https://github.com/mattpocock/skills/commit/3cca18b368ae95cdbdebbff572ccafa662551015)。上游仓库的 package 和 Claude 插件版本均为 `1.2.3`，见 [`package.json`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/package.json) 和 [`plugin.json`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/.claude-plugin/plugin.json)。

## 目录结构和数量

上游 `skills/` 下共有 37 个 `SKILL.md`，分为 18 个稳定工程 skill、7 个稳定 productivity skill、8 个 `in-progress` skill 和 4 个 `misc` skill。`deprecated` 目录当前为空。完整路径由官方 Git tree 的递归结果核对：[pinned tree API](https://api.github.com/repos/mattpocock/skills/git/trees/3cca18b368ae95cdbdebbff572ccafa662551015?recursive=1)。

| 分组 | 数量 | 上游定位 | Claude 插件 | 日常安装脚本 |
|---|---:|---|---|---|
| `skills/engineering` | 18 | 稳定工程流程 | 包含 | 包含 |
| `skills/productivity` | 7 | 稳定通用工作流 | 包含 | 包含 |
| `skills/in-progress` | 8 | Beta，公开试用，可能变化或消失 | 不包含 | 目前仍可链接 |
| `skills/misc` | 4 | 很少使用，不在插件中推广 | 不包含 | 不链接 |
| `skills/deprecated` | 0 | 已退役 | 不包含 | 不链接 |

稳定分组和调用方式来自官方的 [`skills/engineering/README.md`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/README.md) 与 [`skills/productivity/README.md`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/README.md)。`in-progress` 的 Beta 说明和清单见 [`skills/in-progress/README.md`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/README.md)，`misc` 说明见 [`skills/misc/README.md`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/misc/README.md)。

## 稳定工程 skills

官方将这组分成用户主动调用和模型可自动调用两类。用户主动调用项负责组织流程；模型可调用项提供可复用的工程纪律。

| Skill | 调用方式 | 用途 |
|---|---|---|
| [`ask-matt`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/ask-matt/SKILL.md) | 用户主动 | 根据当前情况推荐合适的 skill 或完整流程，是上游 user-invoked skills 的路由器。 |
| [`grill-with-docs`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/grill-with-docs/SKILL.md) | 用户主动 | 通过持续访谈澄清方案，同时建立或更新项目术语、`CONTEXT.md` 和 ADR。 |
| [`triage`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/triage/SKILL.md) | 用户主动 | 按状态机推进 issue，完成分类、补充信息和 Agent brief。 |
| [`improve-codebase-architecture`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/improve-codebase-architecture/SKILL.md) | 用户主动 | 扫描代码库中的模块加深机会，生成可视化 HTML 报告，再针对选中的机会进行讨论。 |
| [`setup-matt-pocock-skills`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/setup-matt-pocock-skills/SKILL.md) | 用户主动 | 一次性配置 issue tracker、Triage 标签和领域文档布局。 |
| [`to-spec`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/to-spec/SKILL.md) | 用户主动 | 将已经讨论过的对话整理成 spec，并发布到配置好的 issue tracker。 |
| [`to-tickets`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/to-tickets/SKILL.md) | 用户主动 | 把计划、spec 或对话拆成 tracer-bullet tickets，并声明 ticket 之间的阻塞关系。 |
| [`implement`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/implement/SKILL.md) | 用户主动 | 按 spec 或 ticket 实现工作，在约定的接缝处驱动 TDD，并在提交前完成 code review。 |
| [`wayfinder`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/wayfinder/SKILL.md) | 用户主动 | 为超过单个 Agent 会话承载能力的大任务建立决策 ticket 地图，逐个解决决策直到路径清晰。 |
| [`prototype`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/prototype/SKILL.md) | 模型可调用 | 用一次性 HTML 原型验证状态模型、逻辑或 UI 设计问题。 |
| [`diagnosing-bugs`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/diagnosing-bugs/SKILL.md) | 模型可调用 | 对难 Bug 和性能回退执行反馈闭环：复现、最小化、假设、加仪表、修复、回归测试。 |
| [`research`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/research/SKILL.md) | 模型可调用 | 基于高可信一手来源调查问题，并把带引用的结果写入仓库 Markdown，按 skill 说明由后台 Agent 执行。 |
| [`tdd`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/tdd/SKILL.md) | 模型可调用 | 用 red-green-refactor 循环，以垂直切片方式实现功能或修复。 |
| [`domain-modeling`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/domain-modeling/SKILL.md) | 模型可调用 | 通过术语和边界场景持续打磨项目领域模型，更新 `CONTEXT.md` 和 ADR。 |
| [`codebase-design`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/codebase-design/SKILL.md) | 模型可调用 | 用深模块、窄接口和清晰接缝改善模块设计与可测试性。 |
| [`code-review`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/code-review/SKILL.md) | 模型可调用 | 从固定基点对 diff 做 Standards 和 Spec 两轴审查，官方流程会并行运行两类 review。 |
| [`resolving-merge-conflicts`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/resolving-merge-conflicts/SKILL.md) | 模型可调用 | 逐个冲突块处理正在进行的 merge 或 rebase，按双方意图解析后完成操作。 |
| [`wizard`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/wizard/SKILL.md) | 模型可调用 | 生成交互式 Bash 向导，引导人完成 Agent 无法代替的凭据、基础设施、第三方后台或一次性迁移步骤。 |

## 稳定 productivity skills

这组是与具体代码库无关的通用工作流工具。

| Skill | 调用方式 | 用途 |
|---|---|---|
| [`grill-me`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/grill-me/SKILL.md) | 用户主动 | 对计划或设计进行持续追问，直到设计树中的分支都得到解决。 |
| [`handoff`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/handoff/SKILL.md) | 用户主动 | 将当前对话压缩成交接文档，供另一个 Agent 继续工作。 |
| [`teach`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/teach/SKILL.md) | 用户主动 | 通过多次会话、使用当前目录作为教学工作区，教用户一个 skill 或概念。 |
| [`to-questionnaire`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/to-questionnaire/SKILL.md) | 用户主动 | 把无法独自回答的决策整理成 Markdown questionnaire，交给能作决定的人异步或共同填写。 |
| [`wait-what`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/wait-what/SKILL.md) | 用户主动 | 当一段说明没有被理解时，结合 `CONTEXT.md` 里的术语重新用直白语言解释。 |
| [`grilling`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/grilling/SKILL.md) | 模型可调用 | 可复用的持续访谈原语，被 `grill-me`、`grill-with-docs`、`triage`、`wayfinder` 等流程使用。 |
| [`writing-for-agents`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/writing-for-agents/SKILL.md) | 模型可调用 | 编写 Agent 会读取的 skills、`AGENTS.md`、`CLAUDE.md` 和其他指针文档。 |

## `in-progress` Beta skills

官方明确说明这组是公开 Beta，不在 Claude 插件和顶层稳定清单中，没有 docs 页面，可能随时变化或消失；需要按名称直接安装。说明和清单见 [`skills/in-progress/README.md`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/README.md)。

| Skill | 用途 |
|---|---|
| [`loop-me`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/loop-me/SKILL.md) | 跨多个会话进行自我追问，把工作流逐步整理成可实现的 spec，并把当前目录作为有状态工作区。 |
| [`writing-beats`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/writing-beats/SKILL.md) | 把文章塑造成由 beats 组成的旅程，每次只写一个 beat，再决定下一步。 |
| [`writing-fragments`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/writing-fragments/SKILL.md) | 通过访谈收集不同类型的写作片段，并追加到一个文档，作为未来文章的原材料。 |
| [`writing-shape`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/writing-shape/SKILL.md) | 读取原始 Markdown 材料，逐段塑造成文章，并讨论每一段的格式选择。 |
| [`claude-handoff`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/claude-handoff/SKILL.md) | 把当前对话交给新的后台 Claude Agent，并通过 handoff 摘要让它立即接手。 |
| [`setup-ts-deep-modules`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/setup-ts-deep-modules/SKILL.md) | 为 TypeScript 仓库接入 dependency-cruiser，把每个 package 约束为隐藏实现、入口可达、通过入口测试的深模块。 |
| [`implement-spec`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/implement-spec/SKILL.md) | 在一个分支实现完整 spec，把 tickets 当作任务图，在 ready frontier 并发运行实现 Agent，最后形成一个 PR。 |
| [`retro`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/in-progress/retro/SKILL.md) | 根据会话提出 Agent 环境改进建议，例如 steering 文件、编码规范、自动检查和工具；当前官方标注为设计说明 stub。 |

## `misc` skills

官方说明这组是作者保留、很少使用且不在插件中推广的工具；当前仓库提交还明确让日常链接脚本跳过 `misc` 和 `deprecated`。来源：[misc README](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/misc/README.md) 和 [`link-skills.sh` 的相关提交](https://github.com/mattpocock/skills/commit/3cca18b368ae95cdbdebbff572ccafa662551015)。

| Skill | 用途 |
|---|---|
| [`git-guardrails-claude-code`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/misc/git-guardrails-claude-code/SKILL.md) | 设置 Claude Code hooks，在危险 Git 命令执行前拦截 push、`reset --hard`、clean 等操作。 |
| [`migrate-to-shoehorn`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/misc/migrate-to-shoehorn/SKILL.md) | 把测试文件中的 `as` 类型断言迁移到 `@total-typescript/shoehorn`。 |
| [`scaffold-exercises`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/misc/scaffold-exercises/SKILL.md) | 创建包含章节、题目、答案和讲解的练习目录结构。 |
| [`setup-pre-commit`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/misc/setup-pre-commit/SKILL.md) | 配置 Husky pre-commit hooks，接入 lint-staged、Prettier、类型检查和测试。 |

## 与 Kite 当前安装的对照

核对对象是 Kite 当前的 [`skills-lock.json`](../../skills-lock.json) 和 [`.agents/skills/`](../../.agents/skills/)。当前本地有 10 个顶层 skill 目录，lock 文件也登记 10 项，二者名称集合一致：

```text
code-review
codebase-design
diagnosing-bugs
domain-modeling
grill-with-docs
grilling
research
setup-matt-pocock-skills
to-spec
triage
```

其中 9 个来自稳定 engineering，`grilling` 来自稳定 productivity。当前安装集合是上游稳定集合的子集；未安装的 27 个包括：

- 稳定 engineering：`ask-matt`、`implement`、`improve-codebase-architecture`、`prototype`、`resolving-merge-conflicts`、`tdd`、`to-tickets`、`wayfinder`、`wizard`。
- 稳定 productivity：`grill-me`、`handoff`、`teach`、`to-questionnaire`、`wait-what`、`writing-for-agents`。
- `in-progress`：`loop-me`、`writing-beats`、`writing-fragments`、`writing-shape`、`claude-handoff`、`setup-ts-deep-modules`、`implement-spec`、`retro`。
- `misc`：`git-guardrails-claude-code`、`migrate-to-shoehorn`、`scaffold-exercises`、`setup-pre-commit`。

`skills-lock.json` 的 10 个条目记录了上游仓库、skill 路径和 `computedHash`，但没有记录上游 package semver 或 commit。因而本报告可以确认当前安装的名称集合和路径集合，不能仅凭该 lock 文件确认本地 10 项是否已经与上游 `1.2.3` 内容逐项同步。

## 对 Kite 委派实现的建议

若目标是把任务交给其他 Agent，优先补装稳定工程里的 [`implement`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/implement/SKILL.md) 和 [`to-tickets`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/to-tickets/SKILL.md)。前者负责从 spec/tickets 进入实现并在提交前收口 review，后者负责把计划拆成带阻塞关系的可认领任务。当前已安装的 `triage`、`to-spec`、`code-review`、`codebase-design`、`domain-modeling` 和 `tdd` 可以组成需求到交付的主链，其中 `tdd` 目前尚未安装，需要单独补装。

跨会话交接可选稳定的 [`handoff`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/productivity/handoff/SKILL.md)；只有整个工作量超过一个 Agent 会话时才考虑 [`wayfinder`](https://github.com/mattpocock/skills/blob/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/wayfinder/SKILL.md)。`implement-spec` 能做更自动化的任务图并发实现，但目前属于 Beta `in-progress`，适合先在隔离分支试用。
