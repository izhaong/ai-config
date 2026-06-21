# ai-config 桌面端 — 产品设计稿

> 状态：Draft v0.2 · 2026-06-21（附录 §19 对齐 Phase 3 已实现 GUI）
> 范围：GUI 信息架构 / 组件清单 / 状态枚举 / 交互流程
> 上游：`PRD.md`（做什么）·`ARCHITECTURE.md`（组件边界）
> 下游：Tauri + Solid 前端代码（不在本文件）
> 形式：文字 + ASCII 线框 + 状态表 + 组件清单 — **不是 Figma**

> **关于"AI 做设计稿"的现实说明**（写给读者也是写给未来的我）：
> AI 不能画 Figma，也不能判断间距 / 配色 / 留白。本文档只覆盖**信息架构层**
> —— 屏幕长什么样、状态有几种、点哪跳哪。视觉层（字号、间距、配色）由人在
> 真实窗口里走查，AI 据此**调 CSS 变量**，不替人做审美决策。
> 低保真可点原型在 `crates/ai-configd-gui/fixtures/` 下，**人**滚动鼠标看完，
> AI 改代码。

---

## 1. 设计原则

1. **三栏是常态，不是变体** — 项目 / 资产类型 / 内容；用户定位一次后不会跳栏
2. **per-item × per-platform 状态徽标是核心** — 一个条目在 **5 个平台**（含 ai-config）什么状态，肉眼能扫
3. **MCP 是表，不是 JSON** — 一行一条 server；JSON 编辑器**不**出现在产品表面
4. **secrets 永不明文** — 任何位置、任何状态、任何字体大小下，明文值不可达
5. **守护进程状态常显** — 用户随时知道 daemon 在不在跑
6. **错误不阻塞** — toast 模式，关键错误才弹模态
7. **diff 必看才让保存** — 编辑器从不"按了 Save 才知道改了什么"

---

## 2. 屏幕清单

| 屏幕 | 入口 | 用途 |
| --- | --- | --- |
| **S1 — 主浏览** | 启动默认 | 三栏浏览 / 编辑入口 |
| **S2 — 编辑器抽屉** | 卡片点编辑 | 编辑 skills / rules / agents 文本 + diff |
| **S3 — MCP 表格** | 中栏点 MCP | 一行一条 server 编辑；右侧 `${VAR}` 联动 |
| **S4 — secrets 列表** | 顶部菜单 / 底部状态栏跳 | key 列表 + 占位 + Set 弹窗 |
| **S5 — 项目注册** | 顶部 "+ Project" | 选 `~/Code/*` 候选项目 |
| **S6 — 同步状态详情** | 卡片平台徽标点 | 单条目 / 单平台同步日志 |
| **S7 — Doctor 卡片** | GUI 启动自检 / 手动触发 | 健康检查 + 修复建议 |
| **S8 — 设置** | 顶部菜单 | 主题 / 守护进程启停 / 关于 |

> 8 个屏幕，**不**再做导航层级（S1 是根，其它都是 modal/drawer/tab）

---

## 3. S1 — 主浏览（三栏）

### 3.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  ai-configd  │  [▾ user-global]  Rules / Skills / MCP / Agents          ⚙ │
│              │  ─────────────────────────────────────────────────────── │
│              │  ● daemon running    branch: develop    ↑0 ↓0            │
├──────────────┼──────────────────────────────────────────────────────────┤
│              │  Type: Skills  (3)  [+]                                  │
│  Projects    │  ┌─────────────────────────────────────────────────────┐ │
│ ──────────   │  │ obsidian-internal-archive              Cc H +X ◯◯◯◯ │ │
│ ● user-global│  │   归档 agent 会话到 ~/Notes/                          │ │
│   (全局)     │  ├─────────────────────────────────────────────────────┤ │
│   zh-cloud   │  │ public-docs-blog-desensitize           Cc H +X ◯◯◯◯ │ │
│   ai-config  │  │   脱敏写 public-docs 博客                             │ │
│   count-web  │  ├─────────────────────────────────────────────────────┤ │
│ + 新项目     │  │ gitea-pr-lifecycle                     Cc H +X ◯◯◯◯ │ │
│              │  │   Gitea PR 收尾                                       │ │
│              │  └─────────────────────────────────────────────────────┘ │
│              │                                                            │
│              │  [▶ 全部同步到当前类型]                                      │
└──────────────┴────────────────────────────────────────────────────────────┘

  平台徽标图例:  ◯ = unlinked   ● = linked   ⊘ = disabled   ⚠ = missing/failed
                 C  Cursor       c  Codex       H  Hermes       X  Claude
```

### 3.2 元素清单

| 元素 | 行为 |
| --- | --- |
| **顶部 — 项目下拉** | `▾` 切换项目；下拉显示所有已注册项目 + "+ 注册新项目" |
| **顶部 — 类型切换** | 4 个 chip：Rules / Skills / MCP / Agents；当前高亮 |
| **顶部 — git 状态** | 分支名 + ahead/behind 徽标；点击跳 S8 或外部 git 工具 |
| **左栏 — 项目列表** | 4 项目 + "+ 新项目"；当前项目高亮 |
| **中栏 — 类型计数** | 名称 + 数量徽标 + 右上角 `+` 新建（仅 Rules / Skills / Agents） |
| **中栏 — 资产卡片** | 名称 / 描述 / 4 平台徽标；点击卡片进 S2 编辑；点击平台徽标进 S6 同步详情 |
| **底部 — 全部同步** | 触发当前类型的"全量同步"到所有启用平台 |
| **底部状态栏** | `● daemon running` / `○ daemon stopped`（**常显**）；点跳 S8 |

### 3.3 平台徽标状态（核心）

| 状态 | 视觉（实现） | 含义 | 点击行为 |
|---|---|---|---|
| **linked** | 实心边框 + 绿高亮 | 本工具下发的硬拷贝（有 marker） | **收回**该平台目录（浏览当前平台时 **skip**） |
| **synced** | 蓝色 partial / 虚线 | 外部安装或手工目录，内容一致 | **覆盖下发** → linked |
| **unlinked** | 空心 + 低透明度 | 该平台无副本 | **下发** / **导入** / **跨平台拷贝** |
| **broken** | 异常态 | marker 或链断裂 | deploy 修复 |
| **missing** | 应对照缺失 | 源侧应有但平台无 | deploy |
| **unsupported** | 灰 + 禁用 | 平台不支持该资产类型 | 无操作 |

平台顺序（固定）：**ai-config** → Cursor → Codex → Claude → Hermes。

列表行右侧控件顺序：**五平台 icon** → **更新（↻）** → **删除（🗑）**。详见 PRD §3.5。

> 下列为 v0.1 线框时代的 4 平台枚举，保留作历史参考；实现以本表与 PRD §3.5 为准。

| 状态 | 视觉 | 触发 | 点击行为（v0.1 草案） |
|---|---|---|---|
| **linked** | `●` 实心 + 平台色 | 软链存在 / JSON 已渲染 | 跳 S6（看日志） |
| **unlinked** | `◯` 空心 + 灰 | 未同步（首次 / 平台关） | 触发单条 sync |
| **disabled** | `⊘` 斜杠 | 该项目下本条目没勾此平台 | 跳 S6（启用） |
| **missing** | `⚠` 红色三角 | 源文件不存在 | 跳 S6（看错误） |
| **failed** | `✕` 红 X | 上次 sync 失败 | 跳 S6（重试） |

**关键设计**：平台徽标横排在卡片右侧，1 秒扫完一个项目所有条目的状态。

---

## 4. S2 — 编辑器抽屉（diff + Save）

### 4.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  ◀ 返回                                       obsidian-internal-archive │
├────────────────────────────┬────────────────────────────────────────────┤
│  editor                    │  diff preview                              │
│  ─────────────────────     │  ─────────────────────                     │
│   1  ---                   │   1  ---                                   │
│   2  name: obsidian-...    │   2  name: obsidian-...                    │
│   3  description: 归档...  │   3  description: 归档...                  │
│   4  ---                   │   4  ---                                   │
│   5                        │   5 + description: 归档到 Obsidian vault   │
│   6  # 步骤                 │   6  # 步骤                                 │
│   7  1. 读会话            │   7  1. 读会话                              │
│   8  2. 解析章节          │   8  2. 解析章节                            │
│   9  3. 写文件            │   9  3. 写文件                              │
│                            │     4. (new) 校验路径权限                  │
│                            │                                            │
├────────────────────────────┴────────────────────────────────────────────┤
│  [Discard]                                              [Save]          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 4.2 行为

- **左侧 editor** — Monaco / CodeMirror 6，语法高亮（markdown / JSON / YAML 看资产）
- **右侧 diff** — 实时（每键入更新一次），git-style 双栏对比
- **Save 按钮** —
  - 始终可点（不算 dirty 也能 save = noop）
  - 保存到 user-global：自动 git commit，commit message 按中文 Conventional Commit 规则弹窗让用户改
  - 保存到 project：只写文件，**不** commit
- **Discard** — 关闭抽屉，未保存改动丢弃（弹确认）
- **校验失败** — Save 灰掉 + 红条说明（kebab-case / JSON parse / 必填字段）

### 4.3 校验规则

| 资产 | 校验 |
|---|---|
| Skill | 目录名 kebab-case；`SKILL.md` 必存在；frontmatter `name` `description` 必填 |
| Rule | 文件名 kebab-case + `.mdc` 结尾；frontmatter `description` 必填 |
| MCP server | name 必填；command/url 二选一；args/env/headers 类型对 |
| Agent | 平台适配器规定（cursor 接受 .md；codex 接受 yaml） |

---

## 5. S3 — MCP 表格（逐项编辑）

### 5.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Type: MCP (12)                              [+ Add Server]             │
├─────────────────────────────────────────────────────────────────────────┤
│  Name           │ Command / URL    │ Vars │ Status                       │
│ ─────────────  │ ─────────────    │ ──── │ ──────                       │
│  ● minio        │ docker run ...   │ ⚠ 3  │ ◯◯◯◯  all linked              │
│  ● obsidian     │ uvx mcp-obsidian │ ⚠ 1  │ ●●●●  all linked              │
│  ● context7     │ https://...      │ ✓ 1  │ ●●●●  all linked              │
│  ○ dart         │ dart mcp-server  │ —    │ ⊘⊘⊘⊘  all disabled            │
│  ✕ jenkins      │ https://...      │ ⚠ 1  │ ⚠⚠⚠⚠  all failed              │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│  minio (selected)                                       [Edit] [Delete]  │
│  ─────────────────────────────────────────────────────────────────────   │
│  command:   docker                                                         │
│  args:      run -i --rm -e MINIO_ENDPOINT ...                              │
│  env:                                                                   │
│    MINIO_ENDPOINT    ${MINIO_ENDPOINT}        ← ⚠ missing in secrets.env  │
│    MINIO_ACCESS_KEY  ${MINIO_ACCESS_KEY}      ← ⚠ missing in secrets.env  │
│    MINIO_SECRET_KEY  ${MINIO_SECRET_KEY}      ← ⚠ missing in secrets.env  │
│    MINIO_USE_SSL     true                                                  │
│                                                                            │
│  Platforms:  ☑ Cursor   ☑ Codex   ☑ Claude   ☑ Hermes                    │
│                                                                            │
│                                                       [Cancel]   [Save]   │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.2 关键设计

- **左侧表** —— 一行一条 server；行点击 → 右侧编辑面板
- **`Vars` 列** —— 显示该 server 的 `${VAR}` 占位符 vs `secrets.env` 实有值的对账
  - `⚠ N` = 有 N 个 `${VAR}` 在 `secrets.env` 中**没**找到值（点击跳 S4 secrets）
  - `✓ N` = 全部找到
  - `—` = server 无变量
- **Status 列** —— 4 个平台徽标（沿用 S1 的设计）
- **右侧编辑面板** ——
  - 表单化（**不**是 JSON 文本框）
  - `${VAR}` 缺失时**内联**红字告警（**不**只"保存后才知道"）
  - Platforms 多选（决定同步到哪几个 IDE）
  - Save 后**只**这条 server 重新渲染到 4 份 `mcp.json`

### 5.3 状态机（MCP server 单条）

```
            [Add Server]                 [Edit]
   (无)  ──────────────►  draft  ──────────────►  editing
                            │   ▲                  │
                            │   └────[Save 失败]──┘
                            ▼
                          saved  ─────[Delete]──►  removed
                            │                      │
                            │ ◄──[undo 5s toast]──┘
                            ▼
                          (事件总线发出 mcp.changed)
                            │
                            ▼
                    (core 重新渲染 4 份 mcp.json)
```

---

## 6. S4 — secrets 列表

### 6.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Secrets                                              [Refresh]         │
├─────────────────────────────────────────────────────────────────────────┤
│  Key                  │ Status   │ Source   │ Updated                    │
│ ────────────────────  │ ──────── │ ──────── │ ────────                   │
│  MINIO_ENDPOINT       │  ● set   │ env      │ 2 days ago                 │
│  MINIO_ACCESS_KEY     │  ● set   │ env      │ 2 days ago                 │
│  OBSIDIAN_API_KEY     │  ● set   │ env      │ 5 days ago                 │
│  JENKINS_AUTH_BASIC   │  ● set   │ env      │ 1 week ago                 │
│  NEW_MCP_KEY          │  ○ unset │ —        │ —                          │
│                                                                            │
│  (5 keys · 1 unset · ⚠ 0 referenced in mcp templates but unset)         │
└─────────────────────────────────────────────────────────────────────────┘

  ┌──────────────────────┐
  │  Set MINIO_ENDPOINT  │
  │  ──────────────      │
  │  ┌────────────────┐  │
  │  │ ••••••••        │  │ ← 占位; focus 时清空
  │  └────────────────┘  │
  │                      │
  │  [Cancel]   [Save]   │
  └──────────────────────┘
```

### 6.2 安全约束（**最高**优先级）

| 规则 | 原因 |
|---|---|
| 列表里**只**显示 "set / unset" 二态，**不**显示值 | 不让屏幕截图泄密 |
| "set" 行**不**允许展开查看值（只允许覆盖改写） | 即使有 viewer 模式也不行 |
| "Set" 弹窗：输入时**不**显示明文（type=password 性质） | 防止录屏 / 旁观 |
| Save 后立即触发 4 份 `mcp.json` 重新渲染 | 用户改完就能用 |
| **不**进 SQLite | 任何 DB 备份 / 同步都带不走 |
| `secrets.env` 文件权限 0600 | 系统层兜底 |
| **不**进日志 / `--json` 输出 | 跨进程边界兜底 |

---

## 7. S5 — 项目注册

### 7.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Register Project                                                         │
├─────────────────────────────────────────────────────────────────────────┤
│  Search ~/Code:                                                          │
│  [zh-cloud_______________]  🔍                                           │
│                                                                            │
│  Detected (have .git, .cursor, .claude, .codex, or .hermes):             │
│  ──────────────────────────────────────────────────────────              │
│  ☑ zh-cloud              ~/Code/zh-cloud                                  │
│      → tools: cursor, codex, claude                                       │
│  ☑ zh-cloud-service      ~/Code/zh-cloud/zh-cloud-service                 │
│      → tools: cursor, claude                                              │
│  ☐ ai-config             ~/Code/ai-config                                 │
│      → tools: cursor, claude, codex, hermes                               │
│  ☐ notes-public          ~/Code/notes-public                              │
│      → tools: —                                                           │
│                                                                            │
│  Selected: 2 projects                                                     │
│                                                                            │
│                                            [Cancel]      [Register]       │
└─────────────────────────────────────────────────────────────────────────┘
```

### 7.2 行为

- 默认扫描 `~/Code/*` 下一级目录
- 候选判定：含 `.git` / `.cursor` / `.claude` / `.codex` / `.hermes` 任一即算
- 每条候选显示**检测到的工具**（读 `.cursor/` 等存在与否）
- 多选 + 一键注册
- 注册后这些项目出现在 S1 左栏

---

## 8. S6 — 同步状态详情

### 8.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  ◀ 返回        obsidian-internal-archive : cursor                        │
├─────────────────────────────────────────────────────────────────────────┤
│  Status: ● linked                                                         │
│  Target: ~/.cursor/skills/obsidian-internal-archive                      │
│  Source: ~/Code/zh-cloud/ai-config/skills/obsidian-internal-archive      │
│  Last sync: 2026-06-12 14:23:01 (2 hours ago)                             │
│                                                                            │
│  Recent events:                                                            │
│  ─────────────────                                                       │
│  14:23:01  sync.finished     status: linked                              │
│  14:22:58  sync.started      platform: cursor                            │
│  14:22:55  watcher.detected  source: SKILL.md modified                   │
│  10:00:12  sync.finished     status: linked                              │
│  09:00:00  daemon.ready                                                  │
│                                                                            │
│  Actions:                                                                 │
│  [Re-sync]  [Disable for this project]  [Open in editor]                  │
└─────────────────────────────────────────────────────────────────────────┘
```

### 8.2 行为

- 展示该 item × 该 platform 的全部状态历史
- 三个 action：
  - **Re-sync** — 强制重做（即使状态是 linked）
  - **Disable for this project** — 改 `enabled` 字段，下次 sync 不再处理
  - **Open in editor** — 跳 S2

---

## 9. S7 — Doctor 卡片

### 9.1 ASCII 线框

```
┌─────────────────────────────────────────────────────────────────────────┐
│  ⚠ Health Check — 3 issues                                                │
├─────────────────────────────────────────────────────────────────────────┤
│  ✕ secrets: OBSIDIAN_API_KEY missing                                     │
│     Referenced by: mcp/servers/obsidian.json                             │
│     Fix: [Open secrets]                                                  │
│                                                                            │
│  ✕ symlink: rules/git-commit-conventions.mdc → ~/.cursor/rules/...      │
│     Status: broken (target was removed)                                  │
│     Fix: [Re-link]                                                       │
│                                                                            │
│  ⚠ project: count-web has no .ai-config/                                 │
│     No per-project overrides registered                                  │
│     Fix: [Create .ai-config/]   [Ignore]                                  │
│                                                                            │
│  All other 12 checks passed                                              │
│  [Re-run doctor]                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 9.2 行为

- 启动时**自动**跑一次；有 issue → 弹 S7 卡片
- 手动触发：底部状态栏 daemon 旁边 `[doctor]`
- 每条 issue 给**具体修复**按钮（不是"自己看着办"）
- 关键 issue（数据可能不一致）红 ✕；建议性 issue 黄 ⚠

---

## 10. S8 — 设置

### 10.1 ASCII 线框（折叠版）

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Settings                                                                  │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│  Daemon                                                                    │
│  ──────                                                                   │
│  Status: ● running (pid 12345)                                            │
│  Socket: ~/.config/ai-config/daemon.sock                                  │
│  [Stop daemon]   [Restart daemon]   [Open daemon log]                     │
│                                                                            │
│  Theme                                                                     │
│  ──────                                                                   │
│  ( ) System   (•) Light   ( ) Dark                                        │
│                                                                            │
│  Notifications                                                             │
│  ──────────────                                                           │
│  ☑ Toast on sync failure                                                  │
│  ☑ Toast on secrets missing                                               │
│  ☐ Toast on every sync (noisy)                                            │
│                                                                            │
│  About                                                                     │
│  ─────                                                                    │
│  ai-configd v0.1.0 (M6 GA)                                                │
│  core: 0.1.0   gui: 0.1.0   daemon: 0.1.0                                │
│  Built: 2026-06-12                                                         │
│                                                                            │
└─────────────────────────────────────────────────────────────────────────┘
```

### 10.2 行为

- 守护进程启停按钮（**不**在 MVP 自动自启，靠用户手动）
- 主题三选一
- 通知开关（控制 toast 频率）
- About 显示版本（agent 排查时截图发我有用）

---

## 11. 组件清单（前端代码组织）

```
crates/ai-configd-gui/src/
├── app.tsx                  # 根：路由 + 全局状态
├── routes/
│   ├── MainBrowser.tsx      # S1
│   ├── EditorDrawer.tsx     # S2
│   ├── McpTable.tsx         # S3
│   ├── SecretsList.tsx      # S4
│   ├── ProjectRegister.tsx  # S5
│   ├── SyncDetail.tsx       # S6
│   ├── DoctorCard.tsx       # S7
│   └── Settings.tsx         # S8
├── components/
│   ├── TopBar.tsx           # 项目下拉 + 类型切换 + git 状态
│   ├── ProjectList.tsx      # 左栏
│   ├── TypeList.tsx         # 中栏类型 chip
│   ├── ItemCard.tsx         # 中栏资产卡片
│   ├── PlatformBadges.tsx   # 4 平台徽标
│   ├── Editor.tsx           # Monaco 包装
│   ├── DiffPanel.tsx        # diff 预览
│   ├── McpRow.tsx           # S3 单行
│   ├── McpEditorPanel.tsx   # S3 右侧编辑
│   ├── Toast.tsx
│   └── Modal.tsx
├── state/
│   ├── store.ts             # Solid signal 全局 store
│   ├── events.ts            # 订阅 daemon 事件流
│   └── tauri.ts             # Tauri command 桥接
├── styles/
│   ├── tokens.css           # 设计 token（颜色 / 间距 / 字号）
│   └── global.css
└── fixtures/                # 低保真可点原型
    ├── index.html
    └── mock-data.json
```

---

## 12. 设计 Token（CSS 变量）

> **这是给人审美的起点**，不是终稿。AI 改代码、人在真实窗口里改 token。

```css
:root {
  /* 颜色 */
  --bg-app:        #fafaf9;     /* 主背景 */
  --bg-panel:      #ffffff;     /* 卡片背景 */
  --bg-muted:      #f4f4f5;     /* 次背景 */
  --border:        #e4e4e7;
  --text-1:        #18181b;     /* 主文字 */
  --text-2:        #71717a;     /* 次文字 */
  --text-muted:    #a1a1aa;

  /* 平台色（与 webui 现有的 tool-icon.tsx 保持一致） */
  --platform-cursor:  #000000;
  --platform-claude:  #D97757;
  --platform-codex:   #10A37F;
  --platform-hermes:  #7C3AED;

  /* 状态色 */
  --status-ok:       #10b981;   /* linked */
  --status-warn:     #f59e0b;   /* missing / unset */
  --status-err:      #ef4444;   /* failed */
  --status-disabled: #a1a1aa;   /* disabled */

  /* 间距 */
  --sp-1: 4px;
  --sp-2: 8px;
  --sp-3: 12px;
  --sp-4: 16px;
  --sp-6: 24px;
  --sp-8: 32px;

  /* 字号 */
  --fs-xs: 11px;    /* 平台徽标 */
  --fs-sm: 12px;    /* 描述 */
  --fs-md: 14px;    /* 卡片标题 */
  --fs-lg: 16px;    /* 类型 chip */
  --fs-xl: 20px;    /* 顶部 */
}
```

> 走查时改这里，**不**改组件内联样式。

---

## 13. 状态机汇总

| 状态对象 | 状态枚举 | 触发 |
|---|---|---|
| **ItemCard** | pending / linked / partial / failed / disabled | 守护进程事件 |
| **McpRow** | draft / editing / saved / removed | 用户操作 |
| **PlatformBadge** | linked / unlinked / disabled / missing / failed | 守护进程事件 |
| **DaemonStatus** | stopped / starting / running / shutting-down | 用户操作 |
| **EditorDrawer** | closed / open-clean / open-dirty / open-invalid | 用户操作 |
| **SecretsList** | loading / ready / saving | 用户操作 |
| **DoctorCard** | idle / running / issue / all-ok | 启动 / 手动触发 |

---

## 14. 关键交互流

### 14.1 启动 → 浏览

```
Tauri 启动
  └─► app.tsx 初始化
        ├─► 调 tauri::command(list_projects)  → store.projects
        ├─► 调 tauri::command(daemon_status)  → store.daemon
        ├─► 调 tauri::command(doctor_run)     → 失败 → 弹 S7
        └─► 渲染 S1
              └─► user-global 默认选中
                    └─► Rules 类型默认选中
                          └─► 拉取该 scope 的 items
                                └─► 渲染卡片
```

### 14.2 编辑 → Save → 自动同步

```
S1 卡片点编辑
  └─► 渲染 S2（拉源文件内容）
        └─► 用户编辑
              └─► 实时 diff 预览
                    └─► Save
                          ├─► 写文件（user-global 自动 commit / project 不 commit）
                          ├─► 调 tauri::command(item_changed)
                          │     └─► 守护进程 200ms 内检测到
                          │           └─► 重新计算 target
                          │                 └─► 应用到 4 平台
                          │                       └─► 发出 sync.finished 事件
                          │                             └─► 前端监听 → 卡片状态更新
                          └─► 关闭抽屉
```

### 14.3 MCP 编辑 → 触发渲染

```
S3 用户编辑一行
  └─► McpEditorPanel.Save
        └─► 写 mcp/servers/<name>.json（原子 rename）
              └─► 调 tauri::command(mcp_changed, name)
                    └─► 守护进程
                          ├─► 校验所有 ${VAR} 在 secrets.env 中
                          │     └─► 缺值 → 发出 secrets.missing 事件（**不**静默）
                          ├─► 合并所有 enabled 条目
                          ├─► 渲染 4 份 mcp.json（tmp + rename）
                          └─► 发出 mcp.rendered 事件
                                └─► 前端：右侧 Vars 列更新 + 状态徽标更新
```

---

## 15. 可访问性 / 国际化（**不在 MVP**）

- 键盘导航：三栏 Tab 切换、卡片 Enter 编辑、Esc 关闭抽屉
- 屏幕阅读器：ARIA labels for 平台徽标（"obsidian-internal-archive: cursor linked, codex linked, claude linked, hermes missing"）
- 高对比模式：跟系统设置走
- i18n：MVP 全中文 + 英文 key（如 `description`）保留；v1.1 加 i18n 框架

---

## 16. 与 webui 的对照

| webui (现状) | 新 GUI |
|---|---|
| 三栏 (项目/类型/内容) | **同** |
| 平台徽标 `bg-xxx` 颜色 | 平台徽标按形状（参考 webui 现有 tool-icon.tsx） |
| MCP JSON 编辑器 | **MCP 表格**（PRD §5.1） |
| secrets bootstrap 一次性按钮 | secrets 列表 + Set/Unset |
| explorer 路由 | 单窗口 + modal/drawer |
| TypeScript + Next.js 15 | Solid + Tauri 2 |
| SQLite DB 在 `webui/data/` | SQLite 在 `~/.local/share/ai-config/` |

---

## 17. 待办（不阻塞 MVP）

- 走查 Tauri 真实窗口 vs ASCII 线框（W8-W9）
- 决定真实字体（VS Code 的 "Inter" / 系统字体 / JetBrains Mono for diff）
- 决定 light / dark 主题切换的 token 第二套值
- 视觉走查：间距 / 对齐 / 留白（人在真实窗口做）
- 国际化 token 文件结构（v1.1 预备）

---

## 18. 变更日志

- **2026-06-12 v0.1** — 初版（来自 PRD v0.2 + ARCHITECTURE.md §11）
- **2026-06-21 v0.2** — §3.3 对齐五平台 + `synced`；附录 §19 记录已实现列表行交互

---

## 19. 附录：Phase 3 已实现 GUI（与线框差异）

当前 `apps/ai-config-gui` 已落地行为（PRD v0.5 §3.5 为权威产品语义）：

### 19.1 布局

| 线框（§3） | 实现 |
| --- | --- |
| 三栏：项目 / 类型 / 内容 | **侧栏**（项目 + 类型 + 五平台浏览）+ **主列表**（工具栏 + 行）+ **抽屉** |
| 4 平台徽标 | **5 平台** favicon 按钮（`PlatformIconButtons`） |
| S6 同步详情页 | **无独立页**；操作在行内完成 + toast |

### 19.2 行内操作（`RowSyncActions`）

```
[☑]  name + description          [ai][Cu][Cx][Cl][He]  [↻]  [🗑]
```

| 控件 | 组件 / 逻辑 |
| --- | --- |
| 五平台 icon | `entryPlatformToggle.ts` → `asset_ops::deploy` / `retract` / `import` / `deploy_from_platform` |
| 更新 ↻ | `entryUpdate.ts` → 对已激活平台批量 `deploy` |
| 删除 🗑 | 二次 arm → `ConfirmModal` → `delete_source`（源视图）或单平台 `retract`（平台视图） |

### 19.3 关键交互规则

1. **五平台对等**：ai-config 目录与其它 IDE 目录均为独立副本；收回 ai-config **不**自动收回 IDE。
2. **浏览当前平台不收回**：`activePlatform === plat` 且非源浏览 / 或源浏览 ai-config 时 skip。
3. **synced**：`platform_scan` + `materialize::is_managed_deploy`；可覆盖为 linked。
4. **跨平台拷贝**：在 IDE 平台 A 视图点平台 B → `deploy_from_platform(A→B)`。

### 19.4 参考实现路径

| 层 | 路径 |
| --- | --- |
| 产品语义 | `docs/product/PRD.md` §3.5 |
| Core | `crates/ai-config-core/src/asset_ops.rs`、`materialize.rs`、`platform_scan.rs` |
| GUI | `apps/ai-config-gui/src/utils/entryPlatformToggle.ts`、`hooks/useAssetOperations.ts` |
| 上游路径 | `docs/reference/vercel-skills-agent-paths.md` |
