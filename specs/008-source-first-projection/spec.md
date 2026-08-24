# Feature Specification: 单一事实源与安全投影

**Feature Branch**: `008-source-first-projection`

**Created**: 2026-07-12

**Status**: Approved（2026-07-16 补齐平台资产契约）
**Input**: 将 agents-manager 作为 skills、rules、commands、agents、入口提示词、hooks 与 MCP 的统一管理源；平台只消费逐项链接或经 adapter 生成的单向投影，解决多份副本、来源竞争、错误平台路径、外部资产误覆盖与 secrets 混入资产仓的问题。

## User Scenarios & Testing

### User Story 1 - 从唯一来源维护并投影资产 (Priority: P1)

作为同时使用多个 Agent/IDE 平台的用户，我希望只在 agents-manager 的统一资产源中维护内容，再将当前作用域解析出的有效资产安全投影到目标平台，从而不再手工维护多份副本。

**Why this priority**: 单一事实源是消除重复、漂移和来源混乱的前提，也是状态、迁移和平台适配的共同基础。

**Independent Test**: 在统一源中添加或修改一个 skill，预览并应用投影；验证支持该资产的平台得到逐项托管引用，平台目录没有生成可独立编辑的内容副本，第二次应用无变更。

**Acceptance Scenarios**:

1. **Given** 全局源中存在合法 skill 且目标平台无同名冲突，**When** 用户预览并应用投影，**Then** 每个支持平台获得指向有效源的逐项托管引用。
2. **Given** 项目覆盖层存在与全局同类型、同名资产，**When** 用户在该项目范围内投影，**Then** 项目资产完整覆盖全局同名资产，且不修改全局源。
3. **Given** 平台不能直接消费某资产格式，**When** 用户应用投影，**Then** adapter 生成该平台可消费的等价表示，同时保留平台中的外部条目和未知字段。
4. **Given** 源与目标均未变化，**When** 用户连续应用两次相同投影，**Then** 第二次不产生文件、链接或配置变化。
5. **Given** 某平台不支持一种资产，**When** 用户预览投影，**Then** 该项显示为 `unsupported`，不得静默跳过或丢字段。

---

### User Story 2 - 只读审计并保护外部资产 (Priority: P1)

作为拥有平台内置插件、手工配置和其他管理工具资产的用户，我希望 agents-manager 能识别它们但默认不接管，使状态检查、同步和卸载都不会破坏现有环境。

**Why this priority**: 当前电脑已经存在 `~/.agents-manager`、`.cc-switch`、平台插件和平台本地目录等多个来源；在无法证明所有权时覆盖或删除，会直接造成数据损失。

**Independent Test**: 准备托管链接、同内容普通副本、异内容同名资产、第三方软链和平台内置资产，运行 `status`、plan、apply 与 uninstall；验证分类准确且所有外部内容保持不变。

**Acceptance Scenarios**:

1. **Given** 平台存在一个没有 agents-manager 所有权证据的资产，**When** 用户运行 `status`，**Then** 系统将其报告为 `foreign`，且不产生任何文件系统或持久状态写入。
2. **Given** 外部资产与源中的同名资产内容完全相同，**When** 系统判断状态，**Then** 可报告 `equivalent`，但不得将其视为托管资产。
3. **Given** 外部资产与源中的同名资产内容不同，**When** 用户预览投影，**Then** 系统报告 `conflict` 并跳过该项，而不是覆盖目标。
4. **Given** 托管链接指向错误来源或生成物被平台侧改写，**When** 用户运行 `status`，**Then** 系统报告 `drifted` 和修复建议，但不自动修复。
5. **Given** 平台聚合配置同时包含托管条目和手工条目，**When** 用户更新或收回托管条目，**Then** 手工条目及其未知字段保持不变。

---

### User Story 3 - 隔离 secrets 并按平台生成 MCP (Priority: P1)

作为管理 MCP server 的用户，我希望资产源只保存可共享的逐 server 配置和 secret 引用，真实 secret 仅保存在专用安全位置，并由平台 adapter 写入 Cursor JSON、Codex TOML、Claude JSON 或 Hermes YAML。

**Why this priority**: 当前单一 `mcp.json` 同时承担源和平台格式，且可能包含明文 env/header；这既不符合平台现状，也会把凭据带入 Git。

**Independent Test**: 使用唯一哨兵 secret 完成 MCP 迁移、预览、投影、状态检查和收回；检查源仓、日志、JSON 输出和 GUI 均不出现哨兵值，同时四平台目标保留其外部配置。

**Acceptance Scenarios**:

1. **Given** `mcp/servers/<name>.json` 使用 secret 引用且安全存储中存在对应值，**When** 用户应用投影，**Then** 各平台获得符合其原生格式的 MCP 条目。
2. **Given** MCP 引用了缺失 secret，**When** 用户预览或应用，**Then** 仅该 MCP 项被阻止，错误只显示缺失 key 名和修复方式。
3. **Given** secret 文件权限过宽，**When** 用户运行 doctor 或应用 MCP 投影，**Then** 系统报告风险并拒绝使用该文件中的值。
4. **Given** 目标配置含外部 MCP 条目，**When** agents-manager 更新或收回一个托管 server，**Then** 只修改该 server，不改动其他条目。
5. **Given** 当前资产根含单体 `mcp.json`，**When** 用户运行 source-first 迁移，**Then** 系统先备份、拆分并校验逐 server 源；明文值只在显式授权后写入 0600 secret 存储并替换为引用。

---

### User Story 4 - 显式导入平台资产 (Priority: P2)

作为希望整理历史资产的用户，我希望先预览平台资产与统一源的差异，再明确选择导入、重命名、替换或取消，从而可控地把有价值的外部资产收敛到统一源。

**Why this priority**: 单向投影不能自动吸收历史资产；显式 import 是唯一受控的反向入口，避免重新引入双向同步。

**Independent Test**: 从某平台选择未托管 skill 导入项目源，验证差异预览、敏感信息检查、同名冲突处理、源侧写入和后续单向语义。

**Acceptance Scenarios**:

1. **Given** 平台存在源中没有的合法外部资产，**When** 用户查看 diff 并明确应用 import，**Then** 系统在选定源层创建规范资产，且不删除平台原件。
2. **Given** 源中已有同名且内容不同的资产，**When** 用户发起 import，**Then** 默认拒绝写入，并要求明确重命名、替换或取消。
3. **Given** 待导入配置含疑似明文 secret，**When** 系统预检，**Then** import 被阻止并提示先转换为 secret 引用。
4. **Given** 外部资产已成功导入，**When** 用户随后直接修改平台侧内容，**Then** 修改不会自动回流源，状态显示漂移或冲突。

---

### User Story 5 - 安全迁移现有混合环境 (Priority: P3)

作为已经拥有大量硬拷贝、旧 marker、旧软链、`.cc-switch` 软链和平台插件资产的用户，我希望先得到完整迁移清单，再分批收敛明确选择的资产，不丢失任何未确认内容。

**Why this priority**: 新架构必须接住现有环境，但迁移应建立在所有权、冲突和投影规则稳定之后。

**Independent Test**: 在临时 HOME 构造旧硬拷贝、marker、正确/错误软链、第三方软链、同名冲突和损坏配置，执行 audit、dry-run 与 apply，验证分类、备份、回滚和幂等。

**Acceptance Scenarios**:

1. **Given** 平台目录中存在多种历史资产，**When** 用户运行迁移审计，**Then** 每项被分类为可迁移、foreign、conflict、drifted 或 unsupported，并显示判定依据。
2. **Given** 历史副本与源内容相同但没有所有权证据，**When** 用户预览迁移，**Then** 仅列为显式接管候选，不自动删除或替换。
3. **Given** 用户应用一批无冲突迁移项，**When** 迁移完成，**Then** 内容先进入并校验统一源，再建立平台投影；未选择项保持不变。
4. **Given** 预览后源或目标发生变化，**When** 用户应用旧计划，**Then** 受影响项以 precondition conflict 失败，不覆盖新状态。
5. **Given** 迁移完成且环境未继续变化，**When** 用户重复审计和预览，**Then** 不再生成重复迁移动作。

### Edge Cases

- 统一源资产缺失、格式无效、名称非法、路径越界或出现循环引用。
- 目标根不存在、不可写、只读、空间不足，或文件/目录类型与预期冲突。
- 平台已有同名外部资产，但内容相同、不同，或仅大小写不同。
- 目标是指向 `.cc-switch`、插件缓存或其他来源的软链。
- 预览完成后、应用前，源或目标的内容、inode 或链接目标发生变化。
- 项目覆盖删除后，应重新解析为全局或 workspace 默认资产，但不得修改全局源。
- 平台配置格式损坏，或包含 adapter 不认识但必须保留的字段。
- MCP 同名但 transport、command、env、headers 或启用状态不同。
- secret 引用缺失、命名非法、权限过宽、值冲突或含特殊字符。
- 一个目标配置同时包含托管、外部和同名冲突条目。
- 项目根已存在非 agents-manager 管理的 `AGENTS.md` 或 `CLAUDE.md`。
- 卸载时托管容器内仍有外部子项。
- 单个平台失败而其他平台已成功。
- Windows 无法创建 Unix 等价软链，需要显式 copy fallback。
- 平台未安装、被禁用或升级后路径/格式发生变化。
- 用户直接把托管链接替换为普通文件，或修改托管生成物。

## Requirements

### 平台资产契约（实现与验收的权威边界）

本节冻结 `skills → rules → MCP → agents → commands → hooks` 的统一源、平台消费位置与作用域语义。实现、迁移、GUI 状态和测试必须以本节为准；当旧 PRD、README、现有 adapter、第三方安装器或本机历史目录与本节冲突时，只能把旧行为列为 legacy inventory，不能继续作为新投影目标。

**官方契约复核日期**：2026-07-16。

**Workspace scope 归一化**：`workspace` 是独立 deploy scope，而非写入用户 HOME 的别名。对表中有稳定 project target 的 Cursor、Codex、Claude 能力，workspace 使用 workspace root 代替 `<repo>`；Hermes 的 workspace skill、MCP 与 Hook 仍为 `unsupported`，不得借此改写全局 `config.yaml`。每个 capability test 必须断言目标仍位于该 workspace root 内。

**上游依据**：

- Cursor：[Skills](https://cursor.com/docs/skills)、[Rules](https://cursor.com/docs/rules)、[Subagents](https://cursor.com/docs/subagents)、[MCP](https://cursor.com/docs/mcp)、[Hooks](https://cursor.com/docs/hooks)、[Commands](https://cursor.com/changelog/1-6)。
- Codex：[Skills](https://developers.openai.com/codex/skills/)、[AGENTS.md](https://developers.openai.com/codex/agent-configuration/agents-md/)、[Execution Rules](https://developers.openai.com/codex/agent-configuration/rules/)、[Subagents](https://developers.openai.com/codex/agent-configuration/subagents/)、[MCP](https://developers.openai.com/codex/extend/mcp/)、[Custom Prompts](https://developers.openai.com/codex/custom-prompts/)、[Hooks](https://developers.openai.com/codex/hooks/)。
- Claude Code：[Directory](https://code.claude.com/docs/en/claude-directory)、[Rules](https://code.claude.com/docs/en/memory#organize-rules-with-clauderules)、[MCP](https://code.claude.com/docs/en/mcp)、[Subagents](https://code.claude.com/docs/en/sub-agents)、[Skills and Commands](https://code.claude.com/docs/en/skills)、[Hooks](https://code.claude.com/docs/en/hooks-guide)。
- Hermes：[Skills](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/skills.md)、[Context Files](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/guides/tips.md)、[MCP](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/mcp.md)、[Hooks](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/hooks.md)。

#### 统一源结构与投影类型

全局源固定为 `~/.agents-manager/`，项目源固定为 `<repo>/.agents-manager/`；两者目录结构必须同构。平台目录不是 source layer。

```text
<asset_root>/
├── skills/<name>/SKILL.md
├── rules/<name>.mdc
├── mcp/servers/<name>.json
├── agents/<name>.md
├── commands/<name>.md
├── prompts/AGENTS.md
├── hooks.json
└── hooks/<single-file-or-bundle>
```

平台 adapter 只能选择以下投影类型：

| 类型 | 含义 |
| --- | --- |
| `direct_link` | 平台能原样消费单条文件或目录；逐资产链接到 canonical source。 |
| `shared_target` | 多个平台官方上会读取同一物理目标；planner 只生成一个 target action，并在状态中列出全部消费者。 |
| `generated` | 平台格式或聚合容器不同；adapter 生成具名条目并保留 foreign/unknown 内容。 |
| `external_directory` | 平台通过官方配置引用一个外部资产根，不复制资产。 |
| `copy_fallback` | 仅平台/OS 无法建立链接且用户显式允许时使用；状态必须显示 `copied`。 |
| `unsupported` | 当前 scope 没有官方消费位置或无法无损表达；不得猜测目录或写入其他 scope。 |

#### 1. Skills

Canonical skill 是 `skills/<name>/` 整目录，`SKILL.md` 必须存在；脚本、参考资料、模板和资源均随该目录作为一个资产单元。禁止链接整个 `skills/` 根。

| 平台 | 用户全局目标 | 项目目标 | 加载/调用语义 | agents-manager 投影决策 |
| --- | --- | --- | --- | --- |
| Cursor | `~/.agents/skills/<name>/` | `<repo>/.agents/skills/<name>/` | 默认按描述自动选择；用户可 `/skill-name`；支持嵌套项目 skills。 | 与 Codex 共用 `shared_target + direct_link`。`.cursor/skills` 仍是官方支持的 alternate location；为避免同一 skill 被投影两次，agents-manager 只盘点/兼容该位置，不作为新默认目标。 |
| Codex | `~/.agents/skills/<name>/` | `<repo>/.agents/skills/<name>/`，按 CWD 到 repo root 发现 | 默认允许 implicit invocation；用户可 `$skill-name`；支持链接后的 skill 目录。 | 与 Cursor 共用 `shared_target + direct_link`。`~/.codex/skills` 和 `<repo>/.codex/skills` 只作 legacy inventory。 |
| Claude | `~/.claude/skills/<name>/` | `<repo>/.claude/skills/<name>/` | 默认按描述自动选择；用户可 `/skill-name`；支持父级、嵌套目录和目录软链。 | `direct_link`。 |
| Hermes | `~/.hermes/config.yaml` 的 `skills.external_dirs` | 无官方 project-scoped skill target | 自动进入 skill index，也可 `/skill-name`；本地 `~/.hermes/skills` 优先于 external dir 同名项。 | 全局为 `external_directory`，指向 canonical global skills root；项目为 `unsupported`，不得把项目 skill 写进全局 Hermes 命名空间。 |

Cursor 与 Codex 对 `.agents/skills` 的可见性是物理共享的：同一个 skill 不能承诺“只对 Codex 可见但对 Cursor 不可见”。GUI 可分别展示两平台兼容性，但 apply/retract 必须折叠为一个共享 target，且收回前必须确认没有其他有效消费者仍请求该 target。

#### 2. Rules

本资产中的 Rule 指“注入模型上下文的指导规则”，不是 shell 权限、审批或执行策略。Canonical rule 使用 `rules/<name>.mdc` 保存正文和可规范化的 description/path/always 语义；adapter 必须无损转换平台触发语义，不能仅改扩展名后假定兼容。

| 平台 | 用户全局目标 | 项目目标 | 格式/加载语义 | agents-manager 投影决策 |
| --- | --- | --- | --- | --- |
| Cursor | 官方 User Rules 存于 Cursor Customize/账号设置，无稳定文件目标 | `<repo>/.cursor/rules/<name>.mdc` | `.mdc`；支持 Always、Agent Decides、路径匹配和手动引用。 | 全局平台投影 `unsupported`；项目 `generated` 或在字节级兼容时 `direct_link`。不得假造 `~/.cursor/rules` 为权威官方目标。 |
| Codex | 无独立 instruction-rule 目录 | 无独立 instruction-rule 目录；项目指导由 `AGENTS.md` 链承载 | `~/.codex/rules/*.rules` 与 `<repo>/.codex/rules/*.rules` 是 Starlark **命令执行策略**，不是提示规则。 | instruction Rule 在全局/项目均为 `unsupported`；需要进入 Codex 的通用指导必须显式纳入 canonical Prompt/AGENTS 方案。绝对禁止把 `.mdc` 写入 `.codex/rules`。 |
| Claude | `~/.claude/rules/<name>.md` | `<repo>/.claude/rules/<name>.md` | Markdown；无 `paths` 时启动加载，有 `paths` 时按文件上下文加载；递归发现并支持软链。 | `generated`；仅当 canonical 内容和 frontmatter 已符合 Claude 契约时可 `direct_link`。 |
| Hermes | 无独立全局模块化 rule 目标 | `<repo>/.cursor/rules/<name>.mdc`（Cursor compatibility） | Hermes 从项目 CWD 读取 `.cursor/rules/*.mdc`；全局身份/人格属于 `SOUL.md`，不等同 Rule。 | 全局 `unsupported`；项目与 Cursor 共用一个 `shared_target`。不得把全局 rule 写入项目或 `SOUL.md`。 |

#### 3. MCP

Canonical MCP 必须是 `mcp/servers/<name>.json` 的逐 server 资产，只保存可共享字段和 secret 引用。平台 MCP 文件都是聚合容器，必须 `generated` merge，禁止软链整份配置或以整文件为所有权单位。

| 平台 | 用户全局目标 | 项目目标 | 平台格式 | agents-manager 投影决策 |
| --- | --- | --- | --- | --- |
| Cursor | `~/.cursor/mcp.json` | `<repo>/.cursor/mcp.json` | JSON `mcpServers` | 按 server 名生成/更新具名条目。 |
| Codex | `~/.codex/config.toml` | `<repo>/.codex/config.toml`（仅 trusted project） | TOML `[mcp_servers.<name>]` | 按 server 名生成/更新 TOML table；不得写 `.codex/mcp.json`。 |
| Claude | `~/.claude.json` 的 user scope | `<repo>/.mcp.json` 的 project scope | JSON `mcpServers` | 全局源映射 user scope，项目源映射可提交的 project scope。Claude local scope 虽也存于 `~/.claude.json`，但属于用户私有项目状态，只盘点、不自动接管。不得写 `~/.claude/mcp.json`。 |
| Hermes | `~/.hermes/config.yaml` 的 `mcp_servers` | 无官方 project-scoped MCP 配置 | YAML | 全局按 server 名生成/更新；项目 `unsupported`，不得为项目请求改写全局 config。 |

同一个目标容器中的 MCP、Hook、Hermes `skills.external_dirs` 等 mutation 必须合并为一个 target batch；整次 plan 只能解析、备份和替换该容器一次。所有平台的 foreign server、注释和未知字段必须保留。

#### 4. Agents / Subagents

Canonical agent 使用 `agents/<name>.md` 表达名称、描述、指令和可移植能力约束。平台格式不同，adapter 必须显式转换；不能把同一 Markdown 盲目复制到所有目录。

| 平台 | 用户全局目标 | 项目目标 | 格式/调用语义 | agents-manager 投影决策 |
| --- | --- | --- | --- | --- |
| Cursor | `~/.cursor/agents/<name>.md` | `<repo>/.cursor/agents/<name>.md` | Markdown + YAML frontmatter；按描述自动委派，也可自然语言指定。Cursor 兼容读取 Claude/Codex agent 目录，但 native 目录优先。 | `generated`；不得默认投影到兼容目录。 |
| Codex | `~/.codex/agents/<name>.toml` | `<repo>/.codex/agents/<name>.toml` | TOML，至少含 `name`、`description`、`developer_instructions`；用户可要求 delegation，规则/skill 也可触发。 | `generated`；禁止使用 `.codex/subagents`。 |
| Claude | `~/.claude/agents/<name>.md` | `<repo>/.claude/agents/<name>.md` | Markdown + YAML frontmatter；Claude 可自动委派，用户可 `@agent-name`。 | `generated`；只有 canonical 已完全符合 Claude schema 时才可 `direct_link`。禁止使用 `.claude/subagents`。 |
| Hermes | 无静态 custom-agent 文件目标 | 无静态 custom-agent 文件目标 | delegation 由运行时和 `config.yaml` 控制，不是逐 agent 文件资产。 | 全局/项目均 `unsupported`；不得创建 `.hermes/agents`。 |

平台不支持的 agent 字段必须在 plan 中显示为 blocking `unsupported` 或明确的降级项；不得静默丢弃 tools、model、readonly/sandbox、skills、MCP 或 delegation 约束。

#### 5. Commands

Canonical command 是 `commands/<name>.md`，语义固定为“用户显式调用的可复用 prompt”。若平台会自动调用同格式内容，adapter 必须加上 explicit-only 控制；command 与 skill 同名时必须在 plan 阶段报告冲突或遵循平台已公开的确定优先级。

| 平台 | 用户全局目标 | 项目目标 | 调用语义 | agents-manager 投影决策 |
| --- | --- | --- | --- | --- |
| Cursor | `~/.cursor/commands/<name>.md` | `<repo>/.cursor/commands/<name>.md`（项目根） | `/name` 显式调用；嵌套项目 commands 不属于当前官方稳定目标。 | `direct_link`；新需求若需要自动判断，应建 Skill 而不是 Command。 |
| Codex | deprecated legacy `~/.codex/prompts/<name>.md` | 无 project-scoped custom prompt | legacy 通过 `/prompts:name` 显式调用；官方要求新可复用 prompt 使用 Skill。 | 全局/项目均 `unsupported`；`.codex/prompts` 只作 migration inventory，可建议转换为 Skill。禁止创建 `.codex/commands`。 |
| Claude | `~/.claude/commands/<name>.md` | `<repo>/.claude/commands/<name>.md` | `/name`；commands 已并入 skills 机制但仍兼容，Skill 同名时 Skill 优先。 | legacy-compatible `direct_link`；新建时 UI 应优先推荐 Skill，但不得擅自改变现有 Command 的显式调用语义。 |
| Hermes | 无独立自定义 command 资产目录 | 无独立自定义 command 资产目录 | 自定义 `/name` 来自 Skill。 | 全局/项目均 `unsupported`；提供“转换为 Skill”建议。 |

#### 6. Hooks

Canonical Hook 由 `hooks.json` 中的具名绑定和 `hooks/` 下的一个合法脚本单元组成。脚本单元只允许“单文件”或“单目录整包”；配置绑定必须由 adapter 合并，脚本/整包可逐项链接。禁止只同步脚本不生成绑定，也禁止只写绑定却引用平台外散落文件。

| 平台 | 用户全局目标 | 项目目标 | 平台格式 | agents-manager 投影决策 |
| --- | --- | --- | --- | --- |
| Cursor | `~/.cursor/hooks.json` + `~/.cursor/hooks/` | `<repo>/.cursor/hooks.json` + `<repo>/.cursor/hooks/` | JSON，camelCase lifecycle events；项目 Hook 可供 Cloud Agent 使用，用户 Hook 不进入 Cloud Agent。 | 配置 `generated` + 脚本单元 `direct_link`。 |
| Codex | `~/.codex/hooks.json` 或 `~/.codex/config.toml` + `~/.codex/hooks/` | `<repo>/.codex/hooks.json` 或 `<repo>/.codex/config.toml` + `<repo>/.codex/hooks/` | JSON/TOML，PascalCase events；项目 Hook 仅 trusted project 加载，并需独立 trust review。 | 新投影固定选择 `hooks.json` 作为单一表示；配置 `generated` + 脚本单元 `direct_link`，不得在同一 scope 同时生成 inline `[hooks]`。 |
| Claude | `~/.claude/settings.json` 的 `hooks` + `~/.claude/hooks/` | `<repo>/.claude/settings.json` 的 `hooks` + `<repo>/.claude/hooks/` | JSON settings，PascalCase events；project/local/user settings 有明确优先级。 | settings 具名绑定 `generated` + 脚本单元 `direct_link`；不得覆盖 permissions、model 等其他设置。 |
| Hermes | `~/.hermes/config.yaml` 的 `hooks` + `~/.hermes/hooks/` | 无官方 project-scoped Hook 配置 | YAML；CLI/gateway 共同加载 shell hooks。 | 全局配置 `generated` + 脚本单元 `direct_link`；项目 `unsupported`，不得为项目请求改写全局 config。 |

Hook adapter 必须完成事件名、matcher、输入输出和阻断语义的显式映射。没有等价事件或返回语义时，该 binding 必须为 `unsupported`，不得仅因“能运行脚本”就声称兼容。

#### 防偏航验收规则

1. 每个平台契约测试必须逐资产断言上表中的 global/project target、格式、投影类型与 unsupported 边界。
2. adapter 不得返回本节未列出的“猜测路径”；legacy 路径只能由 inventory/migration 扫描，不得成为普通 sync 的写目标。
3. 指向同一规范化物理目标的多个平台消费者必须合并为一个 action；状态仍需列出全部消费者及可见性耦合。
4. `supported path` 不等于 `supported format`；无法无损表达字段时必须 generated、降级并提示，或 blocking unsupported。
5. 聚合配置只能按具名 owned entry 修改；任何外部条目、注释、顺序敏感内容和未知字段必须保留。
6. 官方契约 URL 或上游快照变化时，契约测试必须先失败；更新本节和测试快照后才能修改 adapter。
7. 新增资产类型或平台能力前必须先更新本节、对应 FR 与验收用例，禁止先写平台路径再补需求。
8. 契约实现与验收顺序固定为 `Skills → Rules → MCP → Agents → Commands → Hooks`；每类资产必须先补失败测试、再最小实现、跑该类定向测试并留下独立验收记录，上一类未通过不得扩展下一类；每个计划任务完成并验证后形成独立 commit checkpoint。全部本地门禁通过后，才能执行真实 HOME 的只读 inventory；真实写入仅允许用户再次确认的一项无 secret、非关键、无 conflict Skill canary。

### Functional Requirements

#### 单一事实源与作用域

- **FR-001**: 用户级权威源必须为 `~/.agents-manager/`；项目覆盖源必须为 `<repo>/.agents-manager/`；平台目录不得自动成为事实源。
- **FR-002**: 有效资产解析顺序必须固定为 `project override > workspace default > global`，同类型同名采用整项覆盖，异名采用并集。
- **FR-003**: 每次 list、status、plan、apply 和 import 都必须显示资产的实际 source layer 和绝对源路径。
- **FR-004**: MCP server 必须作为独立资产存放于 `mcp/servers/<name>.json`；平台聚合配置不得成为权威源。
- **FR-005**: 入口提示词必须作为 `prompts/<name>.md` 存放在对应 `.agents-manager/` source layer；项目根 `AGENTS.md` 必须是指向 canonical prompt 的逐项链接或可证明所有权的最小生成入口，`CLAUDE.md` 等平台专用入口仅保留 import/reference 与平台差异，不维护多份手工正文。

#### 单向投影与平台适配

- **FR-006**: 自动流向只能是 effective source → platform projection；平台变化不得自动写回源。
- **FR-007**: 仅当平台能原样、无损消费且目标是单条专用路径时，skill、rule、command、agent、prompt 和 hook script 才可逐项链接；异构格式与聚合容器必须生成具名 projection。禁止链接整个平台资产根，也禁止默认创建独立硬拷贝。
- **FR-008**: 平台 adapter 必须明确返回该资产在当前 scope 的 `direct_link`、`generated`、`external_directory`、`copy_fallback` 或 `unsupported` 策略。
- **FR-009**: Skills 必须遵守本节 Skills 契约：Cursor 与 Codex 的 global/repo 投影共用 `.agents/skills` 物理 target，Claude 使用 `.claude/skills`，Hermes global 使用 `skills.external_dirs`；平台可见性耦合必须如实展示。
- **FR-010**: Rules、Agents 与 Commands 必须遵守本节对应契约；Codex instruction rules 不得写入执行策略 `.codex/rules`，Codex/Claude agents 不得写入虚构的 `subagents` 目录，Codex commands 不得写入 `.codex/commands`。
- **FR-011**: MCP 与 Hooks 必须遵守本节对应契约；缺少官方 project-scoped target 的 Hermes project skill、MCP 和 Hook 必须报告 `unsupported`，不得静默写入全局 Hermes 命名空间。
- **FR-012**: Cursor、Codex、Claude 和 Hermes 的聚合配置更新必须保留所有外部条目和未知字段；同一次 plan 中指向同一聚合文件的多个条目必须合并为一个 target batch，只解析、备份和替换一次。
- **FR-013**: Windows fallback 必须显式显示为 `copied`，不得伪装成 `managed_link`；copy 必须经临时路径和 backup-swap，失败时恢复原目标。
- **FR-014**: 在源与目标无变化时重复 apply 必须产生零变更。

#### 所有权、状态与保护

- **FR-015**: 所有权必须来自正确软链目标或 adapter 可证明拥有的具名生成条目；内容相等只能表示 `equivalent`。
- **FR-016**: 核心状态必须为 `managed_link`、`managed_generated`、`copied`、`equivalent`、`foreign`、`conflict`、`drifted`、`missing` 和 `unsupported`；有 ledger/legacy ownership 证据但偏离预期的目标为 `drifted`，没有所有权证据的错源链接为 `foreign`，与期望目标同名且阻塞投影时 plan action 为 `conflict`。
- **FR-017**: 未证明所有权的目标必须视为 `foreign`；同名 foreign 不得被 sync、retract、uninstall 或自动 adopt。
- **FR-018**: `status`、`doctor`、`list` 和 plan 必须严格只读，不得创建目录、迁移 MCP、reconcile hook、写 marker 或更新状态库。
- **FR-019**: retract/uninstall 只能删除仍指向预期源的托管链接或 adapter 拥有的具名生成条目；不得删除普通目录或整份聚合配置。
- **FR-020**: orphan 只能以 report-only 状态报告，并在用户单独请求时生成带所有权证据的 `cleanup_orphan` 计划；不得在普通 sync 中自动删除。

#### Plan、Apply 与 Import

- **FR-021**: install、sync、uninstall、migrate 和批量 import 必须先生成可序列化只读计划，逐项列出 action、source layer、绝对 source path、目标、reason code/说明和 source/target precondition fingerprint；generated batch 必须列出其所有具名 entry intent。
- **FR-022**: apply 前必须重新校验源与目标 fingerprint；变化项必须以 conflict 失败。
- **FR-023**: 同名 foreign 或非法目标默认 conflict；任何接管必须由用户基于当前 plan digest 逐 `action_id` 选择。`equivalent` 可在备份后 adopt；内容不同的 foreign 必须先显式 import/解决 canonical diff，再重新 plan；禁止按平台或来源整批 takeover。
- **FR-024**: 默认 apply 必须是整份 executable plan 原子事务；plan 含 blocking conflict 时不得开始写入。执行失败时必须回滚已执行项，并汇总 `changed`、`unchanged`、`skipped`、`conflict`、`failed`、`rolled_back` 和 `not_applied`，不得把已回滚或未执行项标为成功。
- **FR-025**: import 是唯一平台到源入口，写入前必须展示规范化 diff、目标 source layer 和敏感信息检查结果。
- **FR-026**: import 成功不得删除平台原件，也不得建立双向同步。

#### Secrets 与 MCP

- **FR-027**: canonical MCP 只能保存 secret 引用，agents-manager 不得把真实 token/env/header 值写入资产源或 Git 跟踪文件。
- **FR-028**: 真实 secret 只能保存于 `~/.config/agents-manager/secrets.env`，工具写入后权限必须为 0600。
- **FR-029**: 日志、status、doctor、diff、SQLite、事件、CLI JSON、MCP API 和 GUI 均不得显示 secret 明文。
- **FR-030**: 目标平台必须消费明文时，只能在 apply 阶段写入不可避免的目标配置；渲染结果不得反向作为源。
- **FR-031**: 缺失或权限不安全的 secret 必须把受影响 MCP 项标为 non-executable `skipped`，但不得阻塞不依赖该 key 的其他项；错误仅显示 key 名和修复方式。
- **FR-032**: monolithic `mcp.json` 迁移必须默认 dry-run；显式提取 secrets 时先备份，env 使用原 key，header 使用稳定派生 key，冲突时整项中止。

#### 迁移、仓库卫生与外部来源

- **FR-033**: migration audit 必须识别 legacy marker、legacy agents-manager symlink、无 marker 硬拷贝、错误来源软链、`.cc-switch` 软链、平台 plugin/builtin 和损坏条目。
- **FR-034**: 同内容普通副本只能列为显式迁移候选，不能自动获得所有权。
- **FR-035**: 迁移 apply 必须先备份，再写 canonical source 并校验，最后建立 projection；失败时保留原目标。
- **FR-036**: 工具仓只追踪 canonical 项目资产；生成的 `.cursor/.codex/.claude/.hermes` 平台投影和机器绝对路径不得进入 Git。
- **FR-037**: `.cc-switch`、插件缓存、平台内置资产和其他管理工具必须视为 external；agents-manager 只报告交集和冲突，不擅自清理。
- **FR-038**: 第一阶段不得依赖 daemon、watcher、event bus 或未完成的 SQLite item/target 状态才能保证正确性。
- **FR-039**: CLI、MCP server 与 GUI 必须调用同一 core planner/executor，不得分别实现状态或写入语义；对同一规范化请求，三个入口必须产生相同 schema version、action ordering 和 plan digest。
- **FR-040**: `PlatformCapability` 必须以本节六类资产契约为数据来源，并能区分 source scope、deployment scope、consumer set、target path、format、projection mode、trust requirement 与 legacy inventory path。
- **FR-041**: 对同一物理 target 的共享消费者、同一聚合容器的跨资产 mutation 和同名跨 scope 资产，planner 必须先规范化与去重，再生成唯一 action；不得由平台循环产生重复写入。
- **FR-042**: 所有 legacy/alternate 路径必须在 status 与 migration 中标明来源、是否仍被平台消费以及为何不再作为默认 target；存在 legacy 资产不得让普通 sync 自动迁移、覆盖或删除。

### Key Entities

- **Canonical Asset**: 统一源中的权威资产，kind 包含 skill、rule、command、agent、prompt、hook 和 MCP server；包含稳定 identity、name、source layer、内容与 secret 引用，不含 secret 明文。
- **Source Layer**: `global`、`workspace` 或 `project`；决定作用域与覆盖优先级。
- **Effective Asset**: 在给定项目范围内解析各 source layer 后得到的唯一有效资产。
- **Platform Capability**: 平台在指定 scope 对资产类型的支持模式和原生目标位置。
- **Projection**: Effective Asset 在平台上的托管表现，分为 direct link、generated entry 或 copy fallback。
- **Projection State**: 当前目标的所有权与健康状态，不以内容相等代替所有权。
- **Foreign Artifact**: 平台目标中存在但无 agents-manager 所有权证据的资产或配置条目。
- **Projection Plan**: 只读计算出的 action 集合及其 preconditions；只有显式 apply 才能产生变化。
- **Import Proposal**: 从平台到指定 source layer 的规范化候选，包含 diff、冲突和敏感信息检查。
- **Migration Candidate**: 历史资产盘点结果及建议动作，在显式选择前不具有托管所有权。
- **Secret Reference**: canonical MCP 中的变量引用及解析状态，实体中永不包含明文值。

### Assumptions

- agents-manager 是纳管 skills、rules、commands、agents、入口提示词、hooks 和 MCP 的严格唯一源。
- `prompts/AGENTS.md` 是项目通用入口正文的默认 canonical 名称；现有项目可配置其他名称，但根入口仍只是 projection。
- `.cc-switch` 可继续负责 Provider/账号切换，但不应写入与 agents-manager 相同的托管命名空间。
- 项目同名覆盖采用整项替换，不做字段级自动合并。
- 状态和 plan 始终只读，任何写入都由显式 apply 触发。
- 平台要求保存的已渲染 secret 不属于事实源，且不得反向 import。

### Out of Scope

- 双向实时同步或平台修改自动回流。
- 云端资产托管、远程注册中心或团队权限系统。
- 新增 OS Keychain/1Password secret 后端或自动轮换 token。
- 管理模型、Provider、账号和平台通用 settings。
- 自动合并内容不同的同名资产。
- 自动清理第三方目录或自动重写含历史 secret 的 Git 仓库。
- 在 source-first 正确性稳定前完善 daemon、watcher、event bus。

## Success Criteria

### Measurable Outcomes

- **SC-001**: 在至少 100 个 effective assets × 4 平台的验收集上，100% 受支持项产生正确 projection，0 个不支持字段被静默丢弃。
- **SC-002**: 所有 direct-link 资产只保留一份权威内容；修改 canonical source 后目标立即读取新内容，无需复制同步。
- **SC-003**: 源和目标不变时连续 apply 两次，第二次 changed 数为 0。
- **SC-004**: 在至少 50 个 foreign assets 和 10 个同名 conflict 的环境中执行 status、sync、retract 和 uninstall，外部资产修改/删除数为 0，冲突检出率为 100%。
- **SC-005**: 连续运行 100 次 `list/status/doctor/plan` 前后，受检文件树和配置内容变化数为 0。
- **SC-006**: 对 1,000 个发现项执行审计时，95% 的运行在 2 秒内完成，且每项包含状态原因和允许动作。
- **SC-007**: 使用唯一哨兵 secret 走完迁移、plan、apply、status、doctor、CLI JSON、MCP API 与 GUI 后，源仓、日志和输出中的明文检出数为 0。
- **SC-008**: 工具创建或更新的 secret 文件权限合规率为 100%；权限不合规时受影响 MCP 成功投影数为 0。
- **SC-009**: 在至少 200 个 legacy/foreign/conflict 混合项的迁移验收集中，分类覆盖率为 100%，未选择或冲突资产的数据丢失数为 0。
- **SC-010**: 迁移完成且环境不变时重复 audit/plan，重复 migration action 数为 0。
- **SC-011**: Codex、Claude、Cursor、Hermes 的平台契约测试均使用当前官方目标路径/格式，并在上游契约快照变化时明确失败。
- **SC-012**: 工具仓不再追踪平台 skill 实体副本或含本机绝对路径的生成配置。
