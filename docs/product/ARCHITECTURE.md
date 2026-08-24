# agents-manager 桌面端 — 技术架构

> 状态：Draft v0.1 · 2026-06-12
> 范围：组件 / 模块边界 / 数据流 / 关键决策"为什么"
> 上游：`PRD.md`（做什么）·`SCHEDULE.md`（什么时候做）
> 下游：`.claude/plans/agents-manager/20260612_agents-manager_rust-desktop.plan.md`（具体怎么落地）
> 读者：Code Reviewer / 招人 / 一年后回看"为啥这么选"

---

## 1. 一句话架构

**Rust 单二进制，3 个 crate 协作，1 个守护进程长驻，Tauri 壳包 GUI，Unix socket 接 agent。**

```
                    ┌──────────────────────────────┐
                    │     agents-managerd (单二进制)     │
                    │                              │
   人类 → Tauri  →  │  ┌────────────────────────┐  │  ←─── agent 走
                    │  │     agents-managerd-gui     │  │      Unix socket
                    │  │   (Tauri 2 + Solid)    │  │      (or HTTP)
                    │  └─────────┬──────────────┘  │
                    │            │                 │
                    │  ┌─────────▼──────────────┐  │
                    │  │  agents-managerd-daemon     │  │
                    │  │  (长驻 + 文件监听)      │  │
                    │  └─────────┬──────────────┘  │
                    │            │                 │
                    │  ┌─────────▼──────────────┐  │
                    │  │  agents-managerd-cli        │  │  ←─── agent 调
                    │  │  (clap 11 个子命令)    │  │      agents-managerd <sub> --json
                    │  └─────────┬──────────────┘  │
                    │            │                 │
                    │  ┌─────────▼──────────────┐  │
                    │  │  agents-managerd-core       │  │  ←─── 唯一业务逻辑
                    │  │  (no GUI / no Tauri)   │  │
                    │  └────────────────────────┘  │
                    └──────────────────────────────┘
```

**核心约束**：`agents-managerd-core` 是**唯一**的业务逻辑归宿，CLI / daemon / GUI 都不重复实现，只**调用**。这样 agent 走 CLI 和人类走 GUI 是**同一份代码**跑出来的结果。

---

## 2. 组件清单

| 组件                | 形态                                           | 何时跑     | 谁启动                        |
| ------------------- | ---------------------------------------------- | ---------- | ----------------------------- |
| `agents-managerd-core`   | Rust lib                                       | 编译时     | —                             |
| `agents-managerd-cli`    | Rust bin                                       | 一次性     | 人类 / agent 调               |
| `agents-managerd-daemon` | Rust bin（与 cli 同二进制 `agents-managerd watch`） | 长驻       | 用户手动启 / 未来 launchd     |
| `agents-managerd-gui`    | Tauri 2 应用                                   | 用户开窗时 | 人类点图标 / `agents-managerd gui` |
| **发布形态**        | 单二进制 `agents-managerd` + macOS `.app`           | —          | —                             |

### 2.1 单二进制 vs 多二进制

**决策**：单二进制分发（`agents-managerd`），子命令分发（`install` / `sync` / `watch` / `gui`）。**不**拆多个 binary。

- **理由 1**：安装 / 升级 / 路径管理只一个
- **理由 2**：CLI 和 daemon 共享 core lib 零成本
- **理由 3**：Tauri 2 在 `src-tauri` 嵌一个子进程，**也**能用同一个 binary（见 §6.3）
- **代价**：二进制体量略大（≈ 25MB 含 Tauri runtime），但符合"内部工具 / 一次下载"心智

---

## 3. 模块边界（agents-managerd-core）

```
agents-managerd-core
├── config         # 读 ~/.config/agents-manager/config.toml（per-user 全局配置）
├── model          # 数据类型：Skill / Rule / McpServer / Agent / SymlinkTarget / SyncStatus
├── source         # 资产源解析：扫 skills/ rules/ mcp/ agents/ 目录 → 资产清单
├── platform       # 5 个平台适配器：AgentsManager + Cursor / Codex / Claude / Hermes
│   └── (trait)    #   skills_dir / rules_dir / agents_dir / mcp_deploy_path
├── projection     # source resolver + read-only planner + transactional executor + ownership ledger
├── asset_ops      # source CRUD 与只读详情
├── asset_scope    # 资产根定位、全平台收回（delete_source）
├── platform_scan  # 列表扫描 + per-platform LinkState（含 synced）
├── link           # 遗留：symlink / junction（守护进程 sync 等路径；与 materialize 并存）
├── template       # minijinja 渲染 MCP / secrets 注入 + 原子 rename
├── secrets        # secrets.env 读写（0600），**不**入 SQLite
├── store          # SQLite 持久化：items / targets / events / config
├── sync           # 同步引擎：source + override + enabled platforms → targets
├── watcher        # notify + debouncer-mini（200ms），4 目录
├── bus            # tokio::sync::broadcast 事件总线
├── ipc            # Unix socket 暴露事件流 + 命令（与 daemon 同进程内调）
├── doctor         # 诊断：链接断 / secrets 缺 / 项目未注册
└── error          # thiserror 定义的产品错误 + anyhow 给 CLI 包
```

**硬约束**：

- `core` 模块**不**依赖 Tauri / GUI 任何东西
- `core` 模块**不**用 `Command::new("sh")` / `bash -c` / `sh -c`（PRD §5.3 #2）
- `core` 模块**不**读 secrets 明文到 log（PRD §5.3 #4）

---

## 4. 数据流

### 4.1 全局视角

```
                    ┌─────────────────┐
                    │ 资产源（文件）   │
                    │ ┌─────────────┐ │
                    │ │ skills/     │ │   direct links / generated adapters
                    │ │ rules/      │ │   direct links / generated adapters
                    │ │ mcp/servers/│ │   文件 + 模板渲染
                    │ │ agents/     │ │   硬拷贝
                    │ └─────────────┘ │
                    └────────┬────────┘
                             │  source::scan()
                             ▼
                    ┌─────────────────┐
                    │ core::sync      │
                    │ 计算最终要同步的 │   合并全局 + 项目 .agents-manager/
                    │ 资产清单         │   应用平台开关
                    └────────┬────────┘
                             │ 产生 target actions
                             ▼
                    ┌─────────────────┐
                    │ 4 平台目标       │
                    │ ~/.cursor/      │   symlink / 渲染 JSON
                    │ ~/.codex/       │
                    │ ~/.claude/      │
                    │ ~/.hermes/      │
                    └─────────────────┘
```

### 4.2 守护进程事件流

```
文件系统变化                          SQLite 状态                       agent / GUI
─────────────                      ──────────                      ──────────
notify::Watcher
   │
   │  200ms debounce
   ▼
core::watcher  ──► ItemEvent ──►  core::bus  ──► core::sync  ──►  link::apply
                                                  │
                                                  │  outcome
                                                  ▼
                                          core::store.record()
                                                  │
                                                  │  BusEvent
                                                  ▼
                                          ┌───────┴───────┐
                                          ▼               ▼
                                  Unix socket     Tauri 前端
                                  (agent)         (event listener)
```

### 4.3 MCP 渲染

```
mcp/servers/<name>.json (逐项)          secrets.env (0600, git 外)
┌────────────────────────┐              ┌──────────────────────┐
│ {                      │              │ MINIO_ENDPOINT=...   │
│   "name": "minio",     │              │ OBSIDIAN_API_KEY=... │
│   "command": "docker", │              │ JENKINS_AUTH=...     │
│   "args": [...],       │              └──────────┬───────────┘
│   "env": {             │                         │
│     "MINIO_ENDPOINT": │◄──── minijinja ──────────┘
│       "${MINIO_ENDPOINT}"
│   }
│ }                      │
└──────────┬─────────────┘
           │  core::template::render_many(...)
           │  合并所有 enabled 条目到 mcpServers
           ▼
    ~/.cursor/mcp.json   (原子写：tmp + rename)
    ~/.codex/config.toml (owned MCP tables only)
    ~/.claude.json / <repo>/.mcp.json (owned MCP entries only)
    ~/.hermes/config.yaml  (YAML `mcp_servers:` 段 merge；保留 model/provider 等其它键)
    ~/.hermes/skills/      (全局 skills；项目作用域也写 $HOME)
    <repo>/.cursor/rules/  (Hermes 仅项目级 rules)
```

---

## 5. 平台适配器（核心抽象）

`platform` 模块暴露 **5 个** `PlatformAdapter` 实现：**AgentsManager**（`asset_root` 下目录）+ 4 个 IDE 目标。

```rust
pub trait PlatformAdapter {
    fn id(&self) -> PlatformId;             // "agentsmanager" | "cursor" | "codex" | "claude" | "hermes"
    fn skills_dir(&self) -> PathBuf;         // ~/.agents-manager/skills 或 ~/.cursor/skills 等
    fn rules_dir(&self) -> PathBuf;
    fn agents_dir(&self) -> PathBuf;
    fn mcp_deploy_path(&self) -> PathBuf;    // mcp.json 或 Hermes config.yaml
    fn supports(&self, kind: AssetKind) -> bool;
}
```

GUI 只调用 projection plan/apply/retract 与 reviewed import bridge；executor 根据 ownership ledger 创建 direct links 或受管 generated entries。平台目录从不成为 source。

**上游路径**：IDE skill 目录以 [vercel-labs/skills `src/agents.ts`](https://github.com/vercel-labs/skills/blob/main/src/agents.ts) 为参考；见 `docs/reference/vercel-skills-agent-paths.md`。

**为什么 trait + 多 impl**（vs 纯配置表）：

- 各平台的目录约定**不**是纯路径差异 —— agents 在 Codex 叫 subagents、文件格式可能是 YAML/JSON/MD、mcp.json 里某些字段是平台独有
- 集中在一个 trait 里**未来**支持新平台（Gemini / Zed）= 加一份 impl，**不**改 core

---

## 6. 关键决策（"为什么"）

### 6.1 为啥 Rust（不 Go / 不 Python）

| 维度                                   | Rust         | Go           | Python                |
| -------------------------------------- | ------------ | ------------ | --------------------- |
| 跨平台编译                             | 一等公民     | 一等公民     | 痛                    |
| 单二进制分发                           | 静态链接天然 | 静态链接天然 | 需 PyInstaller / 类似 |
| Tauri 集成                             | 原生         | 不能用 Tauri | 不能用 Tauri          |
| 文件 IO 性能                           | 优           | 优           | 一般                  |
| 学习曲线                               | 陡           | 缓           | 缓                    |
| 与现有 Rust 工具（redis-cli 类似）一致 | ✅           | —            | —                     |

**结论**：Tauri 2 强绑 Rust + 单二进制诉求 → 选 Rust。学习曲线是代价，但**这是技术债不是产品债**。

### 6.2 为啥 Tauri 2（不 Slint / 不 egui / 不 Electron）

| 维度           | Tauri 2      | Slint    | egui        | Electron     |
| -------------- | ------------ | -------- | ----------- | ------------ |
| 二进制大小     | ~10MB        | ~5MB     | ~5MB        | ~100MB       |
| 前端 TS 生态   | 完整         | 无       | 无          | 完整         |
| IPC 类型安全   | ✅           | callback | direct call | IPC          |
| 打包工具成熟度 | 高           | 中       | 中          | 极高         |
| 学习曲线       | 中（前端零） | 中       | 中          | 低（前端熟） |

**结论**：内部工具对二进制大小有要求（10MB 比 100MB 易分发），Tauri 2 同时给到 TS 生态 + 强类型 IPC。egui/Slint 强但前端工作量翻倍，Electron 太重。

### 6.3 为啥 SQLite（不 JSON / 不 sled / 不纯文件）

| 维度                                     | SQLite          | JSON 文件  | sled |
| ---------------------------------------- | --------------- | ---------- | ---- |
| 关系查询（"X 项目下 Y 平台 Z 状态条目"） | ✅              | ❌         | 弱   |
| 并发安全                                 | WAL             | 文件锁     | ✅   |
| 跨进程                                   | ✅              | ❌         | ❌   |
| 体积                                     | ~2MB lib        | 0          | 中   |
| Rust 生态                                | rusqlite / sqlx | serde_json | sled |

**结论**：需要"按项目 × 资产 × 平台 × 状态"维度查 —— 这是关系型查询，SQLite 是显然答案。WAL 模式让 daemon 写 / CLI 读不冲突。

### 6.4 为啥单二进制分发（不 core lib + 多 binary）

单二进制分发是 §2.1 的反面：拆成 `agents-managerd-core.dll` + `agents-managerd.exe` 之类，**安装路径管理复杂、升级要管多个文件、agent 调起要 PATH 配对**。Rust 静态链接 + LTO 后 ≈ 25MB，**不**是负担。

### 6.5 为啥 Unix socket（不 HTTP / 不命名管道 / 不 Tauri IPC 复用）

| 通道                | 谁能连                   | 鉴权             | 跨平台                                |
| ------------------- | ------------------------ | ---------------- | ------------------------------------- |
| **Unix socket**     | 任何本地进程             | 0600 权限 = 鉴权 | macOS/Linux 有，Win 用 `AF_UNIX` 模拟 |
| HTTP localhost:port | 任何能 HTTP 的（含远程） | 需 token         | 三平台都有                            |
| 命名管道            | 同机进程                 | ACL              | Win 原生，Unix 模拟                   |
| Tauri IPC           | 只 Tauri 前端            | Tauri 权限系统   | 仅 Tauri                              |

**结论**：agent 走 socket（独立于 Tauri 进程）；GUI 走 Tauri IPC（绑在 Tauri 进程内）；不互相串。HTTP **不**用（增加鉴权负担、模糊本地/远程边界）。

### 6.6 为啥 minijinja（不 handlebars / 不 tera / 不手写）

MCP 模板就一个文件、< 100 行、变量替换用。minijinja 是 Rust 写的、API 小、二进制 +300KB。**比 handlebars 简单、比 tera 轻**。手写正则**不**考虑（可维护性差，未来加条件渲染就崩）。

### 6.7 为啥 notify + debouncer-mini（不 hotwatch / 不 dnotify）

| crate                       | 跨平台 | debounce     | rename 检测 | 维护 |
| --------------------------- | ------ | ------------ | ----------- | ---- |
| **notify + debouncer-mini** | ✅     | 内置         | ✅          | 活跃 |
| hotwatch                    | ✅     | ❌（自己写） | 弱          | 半弃 |
| dnotify                     | ❌     | —            | —           | —    |

notify 是事实标准，debouncer-mini 是官方推荐搭档。200ms debounce 是行业经验值（IDE 文件保存事件常见双发）。

### 6.8 为啥 Tauri 2 前端选 Solid（不 React / 不 Svelte / 不 Vue）

- **React**：团队熟，但 GUI 体量小（不是 SPA），React 心智模型重了
- **Svelte**：编译期做了很多事，Tauri 2 集成不如 Solid 直接
- **Vue**：与 Solid 接近，团队熟 Vue 但 Tauri 2 模板默认 Solid
- **Solid**：API 表面小，响应式心智跟 Rust 接近（信号 vs 通道），跟 Tauri 2 模板默认

**结论**：Solid 是 Tauri 2 默认前端，与 Rust 心智对齐，体量小。

---

## 7. 持久化模型（SQLite schema 概览）

具体 DDL 在 plan.md §3.1，**这里只讲 4 个核心表的关系**：

```
items
  │  (id, kind: skill|rule|mcp|agent, source_path, scope: global|project:<id>)
  │
  │  1:N
  ▼
targets
  │  (item_id, platform, dest_path, kind: copy|rendered_json, status)
  │  status: linked | synced | unlinked | disabled | missing | failed
  │
  │  1:N
  ▼
events
   (ts, kind, item_id, platform, payload_json)
```

- `items` = 资产清单（"有哪些要管"）
- `targets` = 同步状态（"每条资产在每个平台什么状态"）
- `events` = 事件流（"什么时候发生过什么"）

**`secrets` 表只存元数据**（key、source、updated_at）**不**存值。值永远只在 `secrets.env`。

---

## 8. 错误与日志

### 8.1 错误模型

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("secrets: 缺 {key}（在 {mcp_template}）")]
    SecretsMissing { key: String, mcp_template: PathBuf },

    #[error("链接失败: {dest} → {src}（{reason}）")]
    LinkFailed { src: PathBuf, dest: PathBuf, reason: String },

    #[error("平台 {platform} 不支持资产 {asset}")]
    UnsupportedAsset { platform: PlatformId, asset: AssetKind },

    // ...
}
```

**所有错误带 `hint` 字段**（PRD §6.3），CLI 把它翻译成 stderr 的人话 + JSON 的 `hint` 字段。

### 8.2 日志

- 守护进程：`tracing` + `tracing-subscriber` JSON 输出到 `~/.config/agents-manager/daemon.log`
- 守护进程**不**把 secrets 写入日志（自定义 `tracing-subscriber` layer 过滤 `secrets.*` 字段）
- CLI：`--quiet` 时只输完成/失败；不带 `--quiet` 时输人类可读进度
- GUI：Tauri 前端走自己 console（不开 prod 源）

---

## 9. 安全模型

### 9.1 secrets 边界

| 位置                              | secrets 明文 | 备注                      |
| --------------------------------- | ------------ | ------------------------- |
| `~/.config/agents-manager/secrets.env` | ✅           | 唯一持久位置              |
| 内存中                            | ✅           | 渲染 MCP 时               |
| SQLite                            | ❌           | 只存"是否设置"元数据      |
| 日志（daemon.log）                | ❌           | 过滤器拦                  |
| GUI 屏幕                          | ❌           | `••••••` 占位             |
| CLI stdout                        | ❌           | secrets 子命令默认 redact |
| `--json` 输出                     | ❌           | 同上                      |
| Unix socket                       | ❌           | secrets 不走事件流        |

### 9.2 进程边界

- daemon **不**监听 0.0.0.0（只 Unix socket，本机）
- Unix socket 文件 `0600`，owner = 当前用户
- `agents-managerd uninstall` 之前要 pid file 检查 daemon 是不是在跑

### 9.3 输入校验

- MCP 模板渲染前必 `serde_json::from_str`，parse 失败不写
- `${VAR}` 缺失**告警**而非留空（PRD §5.3 #6）
- 链接目标路径校验：必须**在** `~/.{platform}/` 下，**不**允许软链到 `~` 外（防 `symlink attack`）

---

## 10. 部署形态

### 10.1 macOS

- `tauri-bundler` 出 `.app` + `.dmg`
- 默认安装到 `/Applications/agents-managerd.app`
- CLI 二进制在 `.app/Contents/MacOS/agents-managerd`，symlink 到 `/usr/local/bin/agents-managerd`
- 未来：launchd plist 实现开机自启（不在 MVP）

### 10.2 Linux

- AppImage / deb 二选一（先 AppImage，零依赖）
- CLI 二进制在 `/usr/local/bin/agents-managerd`（deb） 或随 AppImage 走

### 10.3 Windows

- MSI / NSIS 二选一
- CLI 二进制在 `Program Files\agents-managerd\`
- 用户需"开发人员模式"（创建 symlink） 或工具自动 fallback 到 junction
- best-effort，不在 MVP 重点

---

## 11. 跨平台差异表

| 维度            | macOS                  | Linux                                                   | Windows                           |
| --------------- | ---------------------- | ------------------------------------------------------- | --------------------------------- |
| 软链            | symlink                | symlink                                                 | junction（fallback）              |
| Skills 目录     | `~/.cursor/skills` 等  | 同                                                      | `%USERPROFILE%\.cursor\skills` 等 |
| secrets 路径    | `~/.config/agents-manager/` | `~/.config/agents-manager/` 或 `$XDG_CONFIG_HOME/agents-manager/` | `%APPDATA%\agents-manager\`            |
| Unix socket     | ✅                     | ✅                                                      | ❌（用 TCP localhost 或命名管道） |
| `notify`        | 完整                   | 完整                                                    | 完整（junctions）                 |
| Tauri 2 webview | WKWebView（系统自带）  | webkit2gtk（需装）                                      | WebView2（Win10+ 自带）           |

**Windows 抽象层**：

- `link::link()` 在 Windows 自动判断：能用 symlink 就用，否则 junction
- `ipc::serve()` 在 Windows 走 TCP localhost（端口随机或 0，0 = 自动挑空闲）
- `notify` 跨平台统一，配置项差异由 crate 内部处理

---

## 12. 与外部组件的接口

### 12.1 Tauri command（GUI → core）

Tauri 端**不**直接调 core Rust 函数 —— 走 Tauri command bridge：

```rust
// src-tauri/src/commands.rs
#[tauri::command]
async fn list_skills(scope: String) -> Result<Vec<Skill>, String> {
    core::source::scan_skills(&scope).map_err(|e| e.to_string())
}
```

**为什么加这一层 bridge**：Tauri 2 要求所有前端→后端调用走 `#[tauri::command]`，**不**能直接暴露 Rust 函数。这样**未来**加权限校验 / 限流 / 日志都在这一层，不污染 core。

### 12.2 Unix socket 协议（agent → daemon）

帧格式：

```
┌─────────────┬──────────────────────────┐
│ length (4B) │ payload (JSON, UTF-8)    │
│ big-endian  │                          │
└─────────────┴──────────────────────────┘
```

JSON payload：

```json
// request
{ "id": "...", "method": "subscribe", "params": {} }

// response / event
{ "id": "...", "result": {...} }    // 成功
{ "id": "...", "error": { "code": ..., "message": ... } }   // 失败
{ "event": "sync.finished", "data": {... } }                // 推送（无 id）
```

**为什么不复用 Tauri IPC**：Tauri IPC 只在 Tauri 进程内有效；agent 跑在 Cursor/Codex 进程内，**不**能直接连 Tauri。Unix socket 是**对外** IPC，**所有** 进程都能用。

---

## 13. 演进路径（不是当前任务）

未来可能加的（**不**在 MVP 范围）：

- **v0.2** — agents 4 平台全部做完整适配器
- **v0.3** — Windows 体验提升（NSIS 安装器提示开发者模式）
- **v1.0** — OS Keychain 集成（PRD §11 Q-3）
- **v1.1** — i18n（PRD §11 Q-5）
- **v1.2** — 守护进程开机自启（PRD §11 Q-4）
- **v2.0** — 支持新平台（Gemini / Zed / Continue.dev）

每次演进**先**开新 PRD section，**不**直接在架构上叠。

---

## 14. 变更日志

- **2026-06-12 v0.1** — 初版（来自 PRD v0.2 + plan.md §2/§3）
