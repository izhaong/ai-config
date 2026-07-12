# Feature Specification: 单一事实源与安全投影

**Feature Branch**: `008-source-first-projection`

**Created**: 2026-07-12

**Status**: Draft
**Input**: 将 ai-config 作为 skills、rules、commands、agents、入口提示词、hooks 与 MCP 的统一管理源；平台只消费逐项链接或经 adapter 生成的单向投影，解决多份副本、来源竞争、错误平台路径、外部资产误覆盖与 secrets 混入资产仓的问题。

## User Scenarios & Testing

### User Story 1 - 从唯一来源维护并投影资产 (Priority: P1)

作为同时使用多个 Agent/IDE 平台的用户，我希望只在 ai-config 的统一资产源中维护内容，再将当前作用域解析出的有效资产安全投影到目标平台，从而不再手工维护多份副本。

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

作为拥有平台内置插件、手工配置和其他管理工具资产的用户，我希望 ai-config 能识别它们但默认不接管，使状态检查、同步和卸载都不会破坏现有环境。

**Why this priority**: 当前电脑已经存在 `~/.ai-config`、`.cc-switch`、平台插件和平台本地目录等多个来源；在无法证明所有权时覆盖或删除，会直接造成数据损失。

**Independent Test**: 准备托管链接、同内容普通副本、异内容同名资产、第三方软链和平台内置资产，运行 `status`、plan、apply 与 uninstall；验证分类准确且所有外部内容保持不变。

**Acceptance Scenarios**:

1. **Given** 平台存在一个没有 ai-config 所有权证据的资产，**When** 用户运行 `status`，**Then** 系统将其报告为 `foreign`，且不产生任何文件系统或持久状态写入。
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
4. **Given** 目标配置含外部 MCP 条目，**When** ai-config 更新或收回一个托管 server，**Then** 只修改该 server，不改动其他条目。
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
- 项目根已存在非 ai-config 管理的 `AGENTS.md` 或 `CLAUDE.md`。
- 卸载时托管容器内仍有外部子项。
- 单个平台失败而其他平台已成功。
- Windows 无法创建 Unix 等价软链，需要显式 copy fallback。
- 平台未安装、被禁用或升级后路径/格式发生变化。
- 用户直接把托管链接替换为普通文件，或修改托管生成物。

## Requirements

### Functional Requirements

#### 单一事实源与作用域

- **FR-001**: 用户级权威源必须为 `~/.ai-config/`；项目覆盖源必须为 `<repo>/.ai-config/`；平台目录不得自动成为事实源。
- **FR-002**: 有效资产解析顺序必须固定为 `project override > workspace default > global`，同类型同名采用整项覆盖，异名采用并集。
- **FR-003**: 每次 list、status、plan、apply 和 import 都必须显示资产的实际 source layer 和绝对源路径。
- **FR-004**: MCP server 必须作为独立资产存放于 `mcp/servers/<name>.json`；平台聚合配置不得成为权威源。
- **FR-005**: 入口提示词必须作为 `prompts/<name>.md` 存放在对应 `.ai-config/` source layer；项目根 `AGENTS.md` 必须是指向 canonical prompt 的逐项链接或可证明所有权的最小生成入口，`CLAUDE.md` 等平台专用入口仅保留 import/reference 与平台差异，不维护多份手工正文。

#### 单向投影与平台适配

- **FR-006**: 自动流向只能是 effective source → platform projection；平台变化不得自动写回源。
- **FR-007**: 可原样消费的 skill、rule、command、agent、prompt 和 hook script 必须逐项链接；禁止链接整个平台资产根，也禁止默认创建独立硬拷贝。
- **FR-008**: 平台 adapter 必须明确返回该资产在当前 scope 的 `direct_link`、`generated`、`external_directory`、`copy_fallback` 或 `unsupported` 策略。
- **FR-009**: Codex user/repo skills 必须使用官方 `.agents/skills` 路径并支持逐项 symlink；Codex MCP 必须合并到 `.codex/config.toml` 的 `mcp_servers`。
- **FR-010**: Claude skills/agents/commands 与 MCP 必须写入其官方作用域路径；user MCP 不得写入 `~/.claude/mcp.json`。
- **FR-011**: Hermes global skills 优先通过 `skills.external_dirs` 引用 canonical source；在缺少官方 project-scoped target 时，项目 skill、MCP 和 hook 必须报告 `unsupported`，不得静默写入全局 Hermes 命名空间。
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

- **FR-027**: canonical MCP 只能保存 secret 引用，ai-config 不得把真实 token/env/header 值写入资产源或 Git 跟踪文件。
- **FR-028**: 真实 secret 只能保存于 `~/.config/ai-config/secrets.env`，工具写入后权限必须为 0600。
- **FR-029**: 日志、status、doctor、diff、SQLite、事件、CLI JSON、MCP API 和 GUI 均不得显示 secret 明文。
- **FR-030**: 目标平台必须消费明文时，只能在 apply 阶段写入不可避免的目标配置；渲染结果不得反向作为源。
- **FR-031**: 缺失或权限不安全的 secret 必须把受影响 MCP 项标为 non-executable `skipped`，但不得阻塞不依赖该 key 的其他项；错误仅显示 key 名和修复方式。
- **FR-032**: monolithic `mcp.json` 迁移必须默认 dry-run；显式提取 secrets 时先备份，env 使用原 key，header 使用稳定派生 key，冲突时整项中止。

#### 迁移、仓库卫生与外部来源

- **FR-033**: migration audit 必须识别 legacy marker、legacy ai-config symlink、无 marker 硬拷贝、错误来源软链、`.cc-switch` 软链、平台 plugin/builtin 和损坏条目。
- **FR-034**: 同内容普通副本只能列为显式迁移候选，不能自动获得所有权。
- **FR-035**: 迁移 apply 必须先备份，再写 canonical source 并校验，最后建立 projection；失败时保留原目标。
- **FR-036**: 工具仓只追踪 canonical 项目资产；生成的 `.cursor/.codex/.claude/.hermes` 平台投影和机器绝对路径不得进入 Git。
- **FR-037**: `.cc-switch`、插件缓存、平台内置资产和其他管理工具必须视为 external；ai-config 只报告交集和冲突，不擅自清理。
- **FR-038**: 第一阶段不得依赖 daemon、watcher、event bus 或未完成的 SQLite item/target 状态才能保证正确性。
- **FR-039**: CLI、MCP server 与 GUI 必须调用同一 core planner/executor，不得分别实现状态或写入语义；对同一规范化请求，三个入口必须产生相同 schema version、action ordering 和 plan digest。

### Key Entities

- **Canonical Asset**: 统一源中的权威资产，kind 包含 skill、rule、command、agent、prompt、hook 和 MCP server；包含稳定 identity、name、source layer、内容与 secret 引用，不含 secret 明文。
- **Source Layer**: `global`、`workspace` 或 `project`；决定作用域与覆盖优先级。
- **Effective Asset**: 在给定项目范围内解析各 source layer 后得到的唯一有效资产。
- **Platform Capability**: 平台在指定 scope 对资产类型的支持模式和原生目标位置。
- **Projection**: Effective Asset 在平台上的托管表现，分为 direct link、generated entry 或 copy fallback。
- **Projection State**: 当前目标的所有权与健康状态，不以内容相等代替所有权。
- **Foreign Artifact**: 平台目标中存在但无 ai-config 所有权证据的资产或配置条目。
- **Projection Plan**: 只读计算出的 action 集合及其 preconditions；只有显式 apply 才能产生变化。
- **Import Proposal**: 从平台到指定 source layer 的规范化候选，包含 diff、冲突和敏感信息检查。
- **Migration Candidate**: 历史资产盘点结果及建议动作，在显式选择前不具有托管所有权。
- **Secret Reference**: canonical MCP 中的变量引用及解析状态，实体中永不包含明文值。

### Assumptions

- ai-config 是纳管 skills、rules、commands、agents、入口提示词、hooks 和 MCP 的严格唯一源。
- `prompts/AGENTS.md` 是项目通用入口正文的默认 canonical 名称；现有项目可配置其他名称，但根入口仍只是 projection。
- `.cc-switch` 可继续负责 Provider/账号切换，但不应写入与 ai-config 相同的托管命名空间。
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
