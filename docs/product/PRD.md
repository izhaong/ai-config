# ai-config 桌面端 — 产品需求文档 (PRD)

> 状态：**Approved v0.3** · 2026-06-12（开放问题 1–8 已拍板）
> 范围：本产品本身
> 同级：`../../README.md` ·`../../AGENTS.md` ·`../../HERMES.md` ·`../../manifests/plugins.md`
> 技术方案：`.claude/plans/ai-config/20260612_ai-config_rust-desktop.plan.md`

---

## 1. 一句话定位

**写一套（skills / rules / mcp / agents）→ 同步到多个 AI 编码平台。**

- **写一套**：本工具的资产 = 4 种 — 全局在 `global-config/{skills,rules,mcp,agents}/`；项目覆盖在 `<project>/.ai-config/` 下同结构
- **同步到多个平台**：Cursor / Codex / Claude Code / Hermes
- **分层**：全局（user-global，本仓库内容）+ 项目（per-project，项目下 `.ai-config/` 内容覆盖 / 附加）

---

## 2. 资产模型

| 资产       | 物理形态                                         | 一份"项"是什么                                                    | 平台消费形式                             |
| ---------- | ------------------------------------------------ | ----------------------------------------------------------------- | ---------------------------------------- |
| **skills** | 目录（`SKILL.md` + 可选 scripts/assets/agents/） | 一个 skill = 一个目录                                             | 软链 `~/.X/skills/<name>`                |
| **rules**  | 单文件                                           | 一条 rule = 一个 `.mdc` / `.md`                                   | 软链到平台 rules 目录                    |
| **mcp**    | 列表（一项一记录）                               | 一条 mcp server = 一条记录（name + command/args/env/url/headers） | 渲染时合并成 `~/.X/mcp.json`（一份输出） |
| **agents** | 目录或单文件（按平台约定）                       | 一个 agent / subagent = 一个文件或目录                            | 软链到平台 agents 目录                   |

### 2.1 MCP 逐项的关键设计

- **不**再把 `mcp.json` 视作整文件资产。每条 MCP server 是**独立条目**，有 `name`、`command`、`args`、`env`、`url`、`headers`、`enabled` 字段
- 渲染 `~/.X/mcp.json` 时：**所有 enabled 条目** 合并输出到 `mcpServers` 字段
- `${VAR}` 变量从 `secrets.env` 注入；缺失的告警（不留空字符串，避免 IDE 静默坏掉）
- 新增 / 删除 / 改一条 server = 一条 diff，不动其它条目

### 2.2 Agents 的平台差异

各平台对 agent 的叫法、目录约定都不同 —— **本工具把这些差异封装在"平台适配器"里**，对用户暴露的只有"agents 列表"：

| 平台        | 平台内叫法  | 适配器职责         |
| ----------- | ----------- | ------------------ |
| Cursor      | `agents`    | 按平台目录约定链接 |
| Codex       | `subagents` | 同上               |
| Claude Code | `subagents` | 同上               |
| Hermes      | `agents`    | 同上               |

用户**只**关心"我有几个 agent，每个 agent 叫什么、做什么、适用哪些平台"。平台目录叫什么、放哪、文件格式，**适配器**负责。

---

## 3. 同步模型

### 3.1 两层配置

```
┌──────────────────────────────────────────────┐
│  全局（user-global）                          │
│  source = 本仓库 / 全局目录                   │
│  ├─ skills/                                  │
│  ├─ rules/                                   │
│  ├─ mcp/   (逐项)                            │
│  └─ agents/                                  │
└──────────────────────────────────────────────┘
                    +
┌──────────────────────────────────────────────┐
│  项目（per-project, 可选）                    │
│  source = <project>/.ai-config/               │
│  ├─ skills/   (覆盖或附加)                   │
│  ├─ rules/    (覆盖或附加)                   │
│  ├─ mcp/      (覆盖或附加)                   │
│  └─ agents/   (覆盖或附加)                   │
└──────────────────────────────────────────────┘
                    ↓
       计算最终要同步到目标的项集
       （同 name = 覆盖；不同 name = 附加）
                    ↓
┌──────────────────────────────────────────────┐
│  4 个平台目标                                  │
│  Cursor   /   Codex   /   Claude   /   Hermes │
└──────────────────────────────────────────────┘
```

### 3.2 关键行为

- **同 name 覆盖** — 项目下 `skills/foo` 覆盖全局的 `skills/foo`（指向项目源，**不**修改全局）
- **不同 name 附加** — 项目下 `skills/bar` 是项目独有，并入同步集
- **MCP 同 name 覆盖** — 项目下 `mcp/servers/my-srv.json` 覆盖全局同名条目
- **平台级开关** — 每个条目可独立勾选"同步到哪些平台"（`platforms: [cursor, claude, codex, hermes]`）
- **幂等** — 重复跑同步结果一致

### 3.3 持久化

| 数据类型                                   | 存放位置                                   | 理由                                            |
| ------------------------------------------ | ------------------------------------------ | ----------------------------------------------- |
| **资产源**                                 | 文件系统（仓库 + `<project>/.ai-config/`） | 资产本身要进 git 协作                           |
| **同步状态**（per-item × per-platform）    | **SQLite**（守护进程）                     | 关系性查询："哪些项目下哪些条目没同步到 Hermes" |
| **per-platform 勾选**                      | **SQLite**                                 | "skill `foo` 同步到哪些平台"这种状态随项目变化  |
| **事件日志**                               | **SQLite**                                 | 排错用，可选导出                                |
| **secrets**                                | `secrets.env`（0600，git 外）              | **不**入 SQLite、不入仓                         |
| **GUI 配置**（窗口大小 / 主题 / 最近项目） | **SQLite**                                 | 跨机器不必要                                    |

---

## 4. 用户与场景

### 4.1 三类用户

| 用户              | 描述                                         | 主入口          |
| ----------------- | -------------------------------------------- | --------------- |
| **人类开发者**    | 单机多 IDE 用；改一次配置，4 个 IDE 立即见效 | Tauri GUI       |
| **AI 编码 agent** | 在 IDE 内部调起，需要结构化输出              | CLI（`--json`） |
| **首次装机者**    | 全新机器，1 条命令完成所有 IDE 接入          | CLI 一次性命令  |

### 4.2 关键场景

#### A — 全新机器首次接入

1. `git clone ai-config` → 1 条命令 `ai-config install --all`
2. 工具自动：检测已安装 IDE → 软链 skills/rules/agents → 引导 secrets → 渲染 4 份 mcp.json
3. 4 个 IDE 重启即可使用

**验收**：从 clone 到 4 个 IDE 全可见，**只跑一条命令**。

#### B — 添加新 skill

1. 在 `skills/<new>/SKILL.md` 创建
2. 守护进程 < 1s 检测到 → 软链到 4 个 IDE
3. GUI 状态从 "pending" → "linked"

**验收**：从 `mkdir` 到 4 个 IDE 可见，**人类零操作**。

#### C — 添加新 MCP server

1. 在 `mcp/servers/<name>.json`（或 GUI 填表）创建一条记录
2. 守护进程检测到 → 校验 `${VAR}` 都在 `secrets.env` → 重新渲染 4 份 `mcp.json`
3. 重启对应 IDE 即生效

**验收**：缺变量时 CLI/GUI 明确告警，**不**写半个空字符串到 `mcp.json`。

#### D — 项目级覆盖

1. 某项目需要 `skill-foo` 走项目内的版本，**不**影响全局
2. 在 `<project>/.ai-config/skills/skill-foo/` 创建项目版本
3. 工具自动：项目下 `skill-foo` 指向项目源，覆盖全局同名

**验收**：切到该项目工作区，IDE 看到的是项目版本；切到其他项目，看到的是全局版本。

#### E — agent 排查

1. agent 跑 `ai-config doctor --json`
2. 输出结构化诊断："哪个 symlink 断了 / 哪个 secrets 缺 / 哪个项目未注册"
3. agent 据此决策下一步

**验收**：`--json` 全程零人类介入、可解析。

#### F — 卸载 / 回退

1. `ai-config uninstall`
2. 删除本工具创建的所有软链接（**不**动用户手写文件）
3. `mcp.json` 备份为 `mcp.json.ai-config.bak.<ts>`

**验收**：卸载后用户**不丢任何**手写配置。

---

## 5. 功能范围

### 5.1 必做（MVP / GA）

| 模块          | 功能                                                                         |
| ------------- | ---------------------------------------------------------------------------- |
| **CLI**       | `install` / `uninstall` / `sync` / `status` / `doctor` / `list` / `show`     |
| **CLI**       | `mcp` 子命令组（`add` / `remove` / `enable` / `disable` / `list`）— 逐项操作 |
| **CLI**       | `secrets` 子命令组（`bootstrap` / `set` / `list` / `unset`）                 |
| **守护进程**  | `watch` 子命令 — 监听源目录变化、自动同步、暴露事件                          |
| **事件流**    | `events --follow` — agent 实时订阅                                           |
| **Tauri GUI** | 项目注册 / 取消注册                                                          |
| **Tauri GUI** | 浏览 + 编辑（skills / rules / agents）+ diff 预览                            |
| **Tauri GUI** | MCP 表格化编辑（每行一条 server）— **不**是 JSON 编辑器                      |
| **Tauri GUI** | 同步状态（per-item × per-platform 徽标）                                     |
| **Tauri GUI** | secrets 列表（**不**显示明文值）                                             |
| **跨平台**    | macOS 14+ 完整 / Linux (Ubuntu 22.04+) 完整 / Windows 11 best-effort         |

### 5.2 显式不做（非目标）

- ❌ 多租户 / 多用户
- ❌ 云同步（资产走 git，工具只做本地分发）
- ❌ IDE 内部功能（跳转、补全）
- ❌ 插件市场（第三方 skill 直接放目录，工具不审核不评分）
- ❌ 密钥管理（OS Keychain / 1Password）— MVP 用 `secrets.env`，未来 v1.1
- ❌ 远程 MCP 服务管理（MCP 服务跑在哪是用户的事）
- ❌ 多人协作（git 解决）
- ❌ MCP JSON 整文件编辑 — **只**逐项

### 5.3 显式反模式（产品层硬约束）

1. **不**出现 `*.sh` / `*.bash` / `*.mjs` / `*.ps1` / `*.fish`
2. **不**在 Rust 进程内 `Command::new("sh")` / `bash -c` / `sh -c`
3. **不**依赖 Node.js 运行时作为产品运行依赖
4. **不**让 secrets 明文值出现在：日志 / GUI 屏幕 / SQLite / CLI stdout / `--json` 输出 / 任何 HTTP 响应
5. **不**让 GUI 成为 agent 的入口 — GUI 给人类，agent 只走 CLI
6. **不**在 `mcp.json` 写入前不校验 — 必先解析 → 合并 → stringify → 原子写
7. **不**做"事实源分裂"：项目注册 / 平台勾选 / 同步状态只存 SQLite 一处，GUI 和 CLI 共读
8. **不**支持 MCP 整文件资产 — **只**逐项；模板/JSON 编辑不是产品表面

---

## 6. UX 约定

### 6.1 CLI（agent 视角优先）

- **退出码**：0 成功 / 2 参数错 / 3 部分失败 / 4 secrets 缺 / 5 文件系统错
- **默认人类可读**；`--json` 切 JSON；`--quiet` 只输"完成/失败"一行
- **错误从 stderr 出**；`--json` 时 stderr 也是结构化 `{error: {code, message, hint}}`
- **不假设 TTY** — 管道、重定向、子进程调用都 OK
- **`--help` 即文档**

### 6.2 GUI（人类视角）

- **三栏**：左 = 项目列表（含 "user-global"） / 中 = 资产类型（Skills / Rules / MCP / Agents） / 右 = 内容
- **顶部**：项目名 + git 分支 + ahead/behind 徽标
- **每条卡片右侧**：4 个平台的状态指示（linked / unlinked / disabled / missing）+ 单条 sync 按钮
- **MCP 编辑**：表格化（一行 = 一条 server），不暴露原始 JSON；右侧面板显示 `${VAR}` 占位符 vs `secrets.env` 实有值
- **secrets 编辑**：列表 + `••••••` 占位；点 "Set" 才弹输入框
- **底部状态栏**：`● daemon running` / `○ daemon stopped` 常显

### 6.3 错误与故障

- **CLI**：所有错误带 `hint` 字段（修复建议）
- **GUI**：toast 不阻塞；关键错误（同步失败 / secrets 缺）红色高亮 + 跳转对应面板
- **`doctor` 是产品功能**：每次启动 GUI 自检一次，发现问题弹"健康检查"卡片

---

## 7. 验收标准（产品级）

> 可执行 shell / curl / 退出码版本见 plan §5。

- **A-1** 全新 macOS 机器，从 `git clone` 到 4 个 IDE 可见所有资产，**只跑一条** `ai-config install --all`
- **A-2** `ai-config uninstall` 后 `~/.X/mcp.json` 仍存在（备份为 `*.ai-config.bak.<ts>`），用户手写内容完整保留
- **A-3** 安装 / 卸载幂等：跑 10 次 == 跑 1 次
- **A-4** 改 `skills/foo/SKILL.md` 一行，4 个 IDE 的 `skills/foo` 软链目标在 1 秒内反映新内容（守护进程跑着的条件下）
- **A-5** 守护进程未跑时，CLI `sync` 手工补做一次效果一致
- **A-6** 源文件被删 → 4 个 IDE 目标对应软链接自动移除
- **A-7** 加一条 MCP server → 4 份 `mcp.json` 都包含它；缺变量时输出明确告警
- **A-8** 删一条 MCP server → 4 份 `mcp.json` 都不再包含它（**不**残留）
- **A-9** 任意 CLI 子命令 `--json` 都能解析，**不**夹杂人类文本
- **A-10** `doctor --json` 给出 "软链接断在哪 / secrets 缺哪个" 的结构化诊断
- **A-11** 项目下 `.ai-config/skills/foo/` 创建后，该项目工作区里 4 个 IDE 看到的是项目版；其他项目看到的是全局版
- **A-12** `secrets.env` 在所有写入路径上都是 0600
- **A-13** GUI 全屏找不到 secrets 明文值
- **A-14** SQLite DB 中没有任何字段保存 secrets 明文值

---

## 8. 边界

### 8.1 不归本产品管

- IDE 版本兼容（只测 4 款 IDE 最新稳定版）
- Skill 内容质量（评测是 `skills/*/evals/` 的事）
- Git 协作规范（工具只生成 message，规范由 commit hook 管）
- 凭证申请流程（文档里说，不是工具的事）
- MCP 服务本身的可用性（`npx` / `docker` / `uvx` 装不装得上）

### 8.2 与本仓现有内容的关系

| 现有                           | 处置                                          |
| ------------------------------ | --------------------------------------------- |
| `skills/*/SKILL.md`            | 保留，**0 改动** — 资产层                     |
| `rules/cursor/*.mdc`           | 保留，**0 改动** — 资产层                     |
| `mcp/cursor.mcp.template.json` | **重构**为 `mcp/servers/<name>.json` 逐项存储 |
| `mcp/secrets.env.example`      | 保留                                          |
| `manifests/plugins.md`         | 保留                                          |
| `templates/project/`           | 保留（项目脚手架）                            |
| `AGENTS.md` / `HERMES.md`      | 保留                                          |

### 8.3 与 `.claude/plans/` 的边界

- **本 PRD**：产品决策、用户场景、边界、反模式 — **不**写技术选型、CLI flag 细节、SQL DDL
- **plan.md**：技术选型、crate 列表、CLI 子命令完整树、SQLite DDL、4 phase 迁移路径、可执行验收脚本
- 互相引用，不重复内容

---

## 9. 里程碑（状态而非日期）

| 状态                      | 描述                                                                     |
| ------------------------- | ------------------------------------------------------------------------ |
| **M0 — Draft**            | PRD 在 review → **2026-06-12 进入 Approved(开放问题 1–8 拍板,见 §12.4)** |
| **M1 — Approved**         | PRD sign-off，技术方案进入 plan 阶段                                     |
| **M2 — Phase 0 完成**     | 仓库进入 Rust 工具期，行为**未变**（旧 `install.sh` / 旧 webui 仍可用）  |
| **M3 — Phase 1 完成**     | CLI 完全取代旧脚本，行为对齐；`mcp/` 资产从整文件重构为逐项              |
| **M4 — Phase 2 完成**     | 守护进程 + 事件总线工作                                                  |
| **M5 — Phase 3 完成**     | Tauri GUI 完全取代 webui；项目级 `.ai-config/` 适配完成                  |
| **M6 — GA**               | 旧 `install.sh` / 旧 webui 删除；README 更新；MCP 整文件资产下线         |
| **M7 — Deprecate Legacy** | 老命令忽略；旧数据迁完                                                   |

---

## 10. 风险

| 风险                                                                    | 等级 | 缓解                                                          |
| ----------------------------------------------------------------------- | ---- | ------------------------------------------------------------- |
| Tauri 2 仍在快速迭代，半年后 API 可能变                                 | 中   | pin 2.x 稳定大版本，不跟 nightly                              |
| Windows symlink 需要开发者模式 / admin                                  | 中   | 用 `junction` 替代；提示用户                                  |
| `mcp/cursor.mcp.template.json` 整文件 → 逐项重构可能丢用户手改的 server | 中   | 迁移器自动拆 JSON 到 `mcp/servers/*.json`；拆不出来的提示用户 |
| Agent 误跑 `uninstall` 把所有软链删了                                   | 低   | 默认 dry-run；`--apply` 才真删；GUI 二次确认                  |
| secrets.env 误提交                                                      | 低   | 文件在仓外（`~/.config/ai-config/`）；README 强调             |
| Agents 目录约定各平台不同                                               | 低   | 平台适配器封装；用户只关心"我有几个 agent"                    |
| Codex 加载 rules 方式不直接（通过 AGENTS.md 间接消费）                  | 低   | 文档明说；工具不强撑                                          |

---

## 11. 开放问题

> 这些是产品层不确定项；技术不确定项去 plan 写。
> **2026-06-12：以下 8 项已拍板(决策见 §12.4 决策记录);后续新增问题在 plan §8 与本节追加,本表为"产品决策基线"。**

| #   | 问题                                    | 决策(2026-06-12)                  | 决策摘要                                                                                                                                                                                                   |
| --- | --------------------------------------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Q-1 | **分发方式**                            | **三通道并行**                    | Phase 1: `cargo install --path`(仓库内,开发者);Phase 1–2: crates.io(`ai-config = "0.1"`);Phase 3+: 预编译包 `.dmg` / `.AppImage` / `.msi`;Phase 4+: `brew tap zh-cloud/ai-config`(参考 cc-switch 渠道策略) |
| Q-2 | **Tauri 2 前端框架**                    | **React 18**                      | 覆盖 PRD 早期"Solid"推荐;与 plan §2.1 / Phase 3 一致;`apps/ai-config-gui` 锁 `react@18.x`                                                                                                                  |
| Q-3 | **secrets 上 OS keychain**?             | **后续再做**                      | MVP 不做;M7 之后 v1.1 评估;MVP 阶段用 `secrets.env`(0600,git 外)                                                                                                                                           |
| Q-4 | **守护进程开机自启**?                   | **后续再做**                      | MVP 不做;用户 `nohup ai-config watch &`;M6 之后在 `install` 子命令里集成 launchd / systemd --user / Task Scheduler                                                                                         |
| Q-5 | **GUI i18n 先中文还是先英文**?          | **直接中文(因支持中文 IDE 生态)** | 资源文件结构允许两种语言并列,但 MVP 默认中文(目标用户首要是 zh-cloud 团队);v1.1 补英文翻译                                                                                                                 |
| Q-6 | **跨平台支持范围** — FreeBSD / OpenBSD? | **按推荐:否**                     | 只承诺 macOS 14+ / Ubuntu 22.04+ / Win11;不投入 BSD                                                                                                                                                        |
| Q-7 | **agents 适配器优先级**                 | **按推荐:4 平台都做完整**         | adapters 单独配置每个平台的目录约定;`agents/` 资产**不**做平台分目录,只做适配器差异                                                                                                                        |
| Q-8 | **"附加 vs 覆盖"语义** — 项目下同名条目 | **覆盖(按推荐)**                  | 项目源整条覆盖全局;MCP 整条覆盖,**不**字段级合并                                                                                                                                                           |

---

## 12. 附录

### 12.1 FAQ

**Q：跟我的 IDE 设置体系会冲突吗？**
A：本工具只软链 `skills/` / `rules/` / `agents/` 和渲染 `mcp.json`，**不**改 IDE 的 `settings.json` 等。不冲突。

**Q：守护进程占用资源吗？**
A：常驻 5–10MB 内存，闲时 CPU ~0%，文件变化时短暂尖峰。比一个 IDE 小一个数量级。

**Q：能把 secrets 存到 1Password / Keychain 吗？**
A：MVP 不行；v1.1 计划做（Q-3）。

**Q：MCP 整文件模板怎么办？**
A：重构为逐项 `mcp/servers/<name>.json`。迁移器自动拆老模板到逐项文件。

**Q：旧 `install.sh` 删了之后 `git pull` 还能用吗？**
A：M6 之后 `git pull` OK，只是 `install.sh` 不存在了；改用 `ai-config install --all`。M7 之后进入"老命令忽略"。

**Q：项目下 `.ai-config/` 内容会进项目仓吗？**
A：看你 `.gitignore`。工具**不**自动 gitignore；项目仓 owner 自己决定是否进仓。

### 12.2 关键产品决策

| 决策         | 选择                             | 理由                                                             |
| ------------ | -------------------------------- | ---------------------------------------------------------------- |
| 资产 4 种    | skills / rules / mcp / agents    | 覆盖 IDE 实际消费的 4 类内容                                     |
| MCP 形态     | **逐项**，**不**整文件           | 一条 server 一条 diff，缺变量明确告警                            |
| 配置分层     | 全局 + 项目两层                  | 项目能覆盖全局，且不污染全局                                     |
| 平台差异封装 | 适配器模式                       | agents 叫法 / 目录都不同，封装在适配器里                         |
| 事实源       | 文件系统（资产）+ SQLite（状态） | 资产进 git 协作，状态做关系查询                                  |
| IPC          | Unix socket                      | macOS/Linux 都有；Tauri IPC 走前端；agent 走 socket              |
| GUI 打包     | Tauri 2 + React 18               | 二进制小、TS 生态熟、跨平台;React 18 与 plan §2.1 / Phase 3 一致 |
| 模板引擎     | minijinja                        | Jinja2 兼容，零学习成本                                          |
| MCP 渲染     | 全量重写 + 原子 rename           | 简单可靠，几 KB 不在乎                                           |
| 文件监听     | notify + debouncer-mini          | 行业标准，跨平台、debounce 内置                                  |

### 12.3 与外部资产的关系

- **`README.md`** — Quickstart（命令速查）
- **`AGENTS.md`** — Agent 协作规范
- **`HERMES.md`** — Hermes 平台专属
- **`manifests/plugins.md`** — 插件 skill 清单（不入本仓）
- **`templates/project/`** — 项目脚手架
- **`.claude/plans/ai-config/*.plan.md`** — 技术实施方案

### 12.4 产品决策记录(Decision Log)

> 时间倒序;每条引用触发它的开放问题编号。

| 时间       | 决策                                                                                                                        | 触发                          |
| ---------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| 2026-06-12 | **Q-1**:分发方式采用三通道(`cargo install --path` + crates.io + 预编译包),Phase 4 之后增 `brew tap`,参考 cc-switch 渠道策略 | 用户答复                      |
| 2026-06-12 | **Q-2**:Tauri 2 前端锁定 **React 18**(覆盖 PRD 早期"Solid"推荐,与 plan §2.1 / Phase 3 对齐)                                 | 用户答复;消除 PRD ↔ plan 矛盾 |
| 2026-06-12 | **Q-3**:secrets OS keychain **MVP 不做**,v1.1 评估                                                                          | 用户答复                      |
| 2026-06-12 | **Q-4**:守护进程开机自启 **MVP 不做**,M6 后在 `install` 子命令里集成                                                        | 用户答复                      |
| 2026-06-12 | **Q-5**:GUI 默认 **中文**;v1.1 补英文                                                                                       | 用户答复                      |
| 2026-06-12 | **Q-6**:跨平台范围 **macOS 14+ / Ubuntu 22.04+ / Win11**,不支持 BSD                                                         | 用户答复(与 PRD 推荐一致)     |
| 2026-06-12 | **Q-7**:4 平台 agents 适配器 **全部完整实现**;`agents/` 资产不分平台目录                                                    | 用户答复(与 PRD 推荐一致)     |
| 2026-06-12 | **Q-8**:项目下同名条目 **整条覆盖**(MCP 不字段级合并)                                                                       | 用户答复(与 PRD 推荐一致)     |
