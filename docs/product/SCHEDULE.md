# ai-config 桌面端 — 项目排期

> 状态：**v0.2** · 2026-06-12（M1 Approved；Q-1..Q-8 拍板同步；crate 命名/前端框架与 plan 对齐）
> 范围：本项目交付的时间 / 任务 / 依赖 / 资源
> 上游：`PRD.md` v0.3 Approved（§12.4 决策日志）·`.claude/plans/ai-config/20260612_ai-config_rust-desktop.plan.md`（§8.1 已决 / §8.2 待决）
> 下游：`ARCHITECTURE.md`（组件 / 模块边界）·`DESIGN.md`（GUI 信息架构）
> 修订：每周末更新（一次大迭代后大改一次）
> **CLI 二进制名**：`ai-config`（2026-06-12 统一；旧 `ai-configd` 措辞已废止，参见 PRD §6.1 退码表）

---

## 1. 排期约定

- **W 编号**：W0 = 当前周（2026-06-09 ～ 2026-06-15），向后递增
- **颗粒度**：任务 = 4–16 小时（约 1–2 天），超过 16 小时要拆
- **依赖图**：用 → 表示，X → Y 表示 X 完成后才能开始 Y
- **优先级**：P0 = 不做就 GA 不能发布；P1 = GA 之后第一个 minor；P2 = 攒着
- **人天**：1 人天 = 6 小时净开发（不含会议 / 编译等待 / review）
- **状态**：⬜ 未开始 / 🟡 进行中 / ✅ 完成 / ⛔ 阻塞
- **窗口**：除特别说明，**单线程**顺序执行（"AI 工具 + 1 个开发者"模式，不预设多人）

---

## 2. 里程碑日历（与 PRD §9 对齐）

| 里程碑 | 预计 W | 状态 | 关键交付 |
| --- | --- | --- | --- |
| M0 — Draft PRD | W0（已完成） | ✅ | `PRD.md` 落盘 |
| M1 — Approved | W1 | ⬜ | PRD sign-off；plan / arch / schedule 落盘 |
| M2 — Phase 0 完成 | W2 | ⬜ | `crates/` workspace + CI + pre-commit 禁 sh + 旧命令仍可用 |
| M3 — Phase 1 完成 | W5 | ⬜ | CLI 取代 `install.sh` + `scripts/*.mjs`；MCP 整文件 → 逐项重构完成 |
| M4 — Phase 2 完成 | W7 | ⬜ | 守护进程 + 事件总线 |
| M5 — Phase 3 完成 | W10 | ⬜ | Tauri GUI 取代 webui；项目级 `.ai-config/` 适配 |
| M6 — GA | W11 | ⬜ | 旧脚本/webui 删除；README 更新；`SCHEDULE.md` 进入"维护期" |
| M7 — Deprecate Legacy | W23（GA + 3 月） | ⬜ | 老命令完全忽略；旧 DB 迁完可删 |

> 总工时估算见 §3；当前只到 W1，**W2 起的 W 编号是预估**，实际每周日 review 后更新。

---

## 3. 工时估算（按 phase）

> 数字 = 人天。**带 ±20% 误差**。如果实际偏离 50% 以上，回头校准这个表。

| Phase | 子任务群 | 人天 | 关键风险点 |
| --- | --- | --- | --- |
| **0 — 准备** | crates workspace | 0.5 | — |
| | CI 配置（cargo test / clippy / fmt） | 0.5 | — |
| | pre-commit 禁 sh / 禁 mjs | 0.5 | 钩子误报 |
| | rust-toolchain.toml + 跨平台预编译调研 | 1.0 | tauri-bundler 在 Win 编译链 |
| | **小计** | **2.5** | |
| **1 — CLI** | `ai-config-core` 模块（fs / 模板 / 链接） | 4.0 | junction 跨平台 |
| | `ai-config-cli` 子命令树（11 个顶层） | 3.0 | clap 派生 + 退出码 |
| | `mcp` 逐项子命令组 | 2.0 | 旧整文件 → 逐项迁移器 |
| | `secrets` 子命令组 | 1.5 | 0600 写入 + 引导 |
| | 行为对齐测试（旧 install.sh vs 新 CLI） | 2.0 | 跨平台 fixture |
| | **crates.io 发布准备**（Q-1 第二通道）：`cargo login`、`cargo publish --dry-run`、发布 `ai-config = "0.1"`、验证 tarball（Q-1 决策：Phase 1–2 之间上线） | 0.5 | crates.io 账号 + API token 需用户提供 |
| | **小计** | **13.0** | |
| **2 — 守护进程** | `notify` watcher + debouncer | 2.0 | rename 检测 |
| | 事件总线（broadcast channel） | 1.0 | — |
| | Unix socket 暴露给外部订阅者 | 2.0 | 鉴权（要吗？） |
| | `events --follow` CLI | 1.0 | — |
| | 自启动调研（macOS launchd / systemd-user）— 调研 + 文档 | 1.0 | 调研不实现（实现推到 Phase 4 / Q-4） |
| | **小计** | **7.0** | |
| **3 — Tauri GUI** | 空白 Tauri 2 + React 18 脚手架（Q-2 拍板） | 1.0 | 跨平台 webview 差异 |
| | 三栏布局 + 路由 | 2.0 | — |
| | 项目注册 / 取消注册 | 1.5 | — |
| | skills / rules 浏览 + 编辑器 + diff | 3.0 | Monaco 在 webview 的体积 |
| | **MCP 表格化编辑**（每行 = 一条 server，**不**用 JSON 编辑器） | 3.0 | `${VAR}` 高亮联动 |
| | agents 浏览 + 编辑（Q-7：4 平台适配器全做） | 2.0 | 上调 0.5 天（4 适配器 vs 原估 2 平台） |
| | 同步状态徽标 + 单条 sync 按钮 | 2.0 | — |
| | secrets 列表（**不**显示明文） | 1.0 | — |
| | DB 迁移器（webui → 新 SQLite） | 1.5 | schema 漂移 |
| | **i18n 资源骨架**（Q-5：MVP 默认中文）：i18next 或 react-intl 二选一，资源文件 `zh-CN.json` 落盘，留英文空 key | 0.5 | 与 GUI 同时落地 |
| | **预编译包 CI**（Q-1 第三通道）：GitHub Actions / Gitea Actions matrix（macos-13/14 arm64+x64、ubuntu-22.04、windows-2022），用 `tauri-action` 出 `.dmg`/`.AppImage`/`.msi`；macOS 签名 + 公证 | 1.5 | runner 预算 ~25 min/平台/次；需 Apple Developer 账号 |
| | **小计** | **19.0** | |
| **4 — 清理** | 删 `install.sh` / `scripts/` / `webui/` | 0.5 | 验证旧命令不再被引用 |
| | README 更新（命令清单） | 0.5 | — |
| | `AGENTS.md` / `HERMES.md` 命令引用同步 | 0.5 | — |
| | `skills/*/SKILL.md` 中提到 `install.sh` 的全部更新 | 1.0 | 全文 grep |
| | 发版说明（draft for v0.1） | 0.5 | — |
| | **brew tap 发布**（Q-1 第四通道）：建 `homebrew-zh-cloud` 仓，写 `ai-config.rb` Formula，README 加 `brew install zh-cloud/ai-config/ai-config` 横幅 | 0.5 | 需先发 GH Release |
| | **自启动集成子命令**（Q-4 决策：M6 之后在 `install` 子命令里集成 launchd / systemd --user / Task Scheduler）：`ai-config install --autostart` 子模式 | 1.0 | launchd plist / systemd user unit 模板 |
| | **小计** | **4.5** | |

**总计**：≈ **46.0 人天**（净 7.7 周 / 6h·d⁻¹），含 30% 缓冲 ≈ **11 周**。与 §2 里程碑 M6 ≈ W11 一致。

---

## 4. W-by-W 拆解

> W0/W1 已发生或正在发生。W2 起是计划，**每周日 review 后回写实际**。

### W0 (2026-06-09 ~ 06-15) — Draft PRD ✅

- [x] PRD v0.1 落盘
- [x] 跑完 Rust port workflow（5 phase，7 subagent）
- [x] PRD v0.2 修订（加 agents / MCP 逐项 / 两层配置）

### W1 (2026-06-16 ~ 06-22) — Plan & Arch & Design

- [ ] **M1** PRD sign-off
- [x] PRD §12.4 决策日志：Q-1..Q-8 拍板（2026-06-12；分发三通道 / React / keychain 后续 / 自启后续 / 中文 i18n / 不支持 BSD / 4 平台 agents / 整条覆盖）
- [ ] `SCHEDULE.md`（本文档）落盘
- [ ] `ARCHITECTURE.md` 落盘
- [ ] `DESIGN.md` 落盘
- [ ] 同步 plan.md §8.1 把 Q-1/Q-2 标已决，§8.2 保留 Q-3..Q-6 待决（2026-06-12 已落）

### W2 (2026-06-23 ~ 06-29) — Phase 0 准备

- [ ] 创 `crates/` workspace（**7 个 crate，对齐 plan §2.2.1**）：
  - `crates/ai-config-core`、`crates/ai-config-cli`、`crates/ai-config-daemon`
  - `crates/ai-config-bus`、`crates/ai-config-watcher`、`crates/ai-config-store`
  - `apps/ai-config-gui`（Tauri 壳，Phase 3 才起骨架；本阶段只建空目录）
- [ ] 根 `Cargo.toml` workspace 片段
- [ ] CI workflow（GitHub Actions / Gitea Actions 选一）：`cargo test` + `cargo clippy -- -D warnings` + `cargo fmt --check`
- [ ] Pre-commit 钩子：`git grep -E '\.sh$|\.mjs$|install\.sh' -- ':!**/legacy/**'` 在 commit 前 fail
- [ ] `.gitignore` 加 `target/` `.ai-config/daemon.sock`
- [ ] `rust-toolchain.toml` pin stable
- [ ] 调研 tauri-bundler 跨平台编译链（macOS / Linux / Windows），记录到 plan.md

**Phase 0 exit criteria**：`cargo build` 通过；pre-commit 钩子跑通；旧 `install.sh` / 旧 webui **行为未变**还能用。

### W3 (2026-06-30 ~ 07-06) — Phase 1.a core lib

- [ ] `ai-config-core` 模块：`fs`（链接 + junction 抽象）
- [ ] `ai-config-core` 模块：`template`（minijinja + 原子 rename）
- [ ] `ai-config-core` 模块：`config`（TOML / serde）
- [ ] `ai-config-core` 模块：`error`（thiserror + anyhow）
- [ ] 单元测试：链接幂等 / 模板变量替换 / 0600 写入

### W4 (2026-07-07 ~ 07-13) — Phase 1.b CLI

- [ ] `ai-config-cli`：clap 派生 11 个顶层子命令
- [ ] `install` / `uninstall` / `sync` / `status` / `list` / `show` / `doctor`
- [ ] 全局 `--json` / `--quiet` 实现
- [ ] 退出码 0/2/3/4/5
- [ ] 集成测试：干净 `$HOME` 下，`./install.sh --all` 与 `ai-config install --all` 行为对齐

### W5 (2026-07-14 ~ 07-20) — Phase 1.c MCP 逐项 + 迁移

- [ ] `mcp add/remove/enable/disable/list` 子命令
- [ ] `mcp` 整文件 → 逐项迁移器（读 `mcp/cursor.mcp.template.json`，拆成 `mcp/servers/<name>.json`）
- [ ] `secrets bootstrap` / `set` / `list` / `unset`
- [ ] **M3 — Phase 1 完成**

**Phase 1 exit criteria**：4 项 A-1 / A-3 / A-7 / A-8（PRD §7）验收过；旧 `install.sh` 退化为一行 wrapper（`exec cargo run -p ai-config-cli --quiet -- install "$@"`），`scripts/*.mjs` 删除。

### W6 (2026-07-21 ~ 07-27) — Phase 2.a 守护进程核心

- [ ] `ai-config-daemon` crate
- [ ] `notify` watcher + `notify-debouncer-mini`（200ms）
- [ ] 监听 `skills/`、`rules/cursor/`、`mcp/servers/`、`agents/` 四个目录
- [ ] 事件总线：`tokio::sync::broadcast` channel
- [ ] 写 4 个平台目标（用 Phase 1 的 core 链接函数）

### W7 (2026-07-28 ~ 08-03) — Phase 2.b IPC + agent CLI

- [ ] Unix socket：`~/.config/ai-config/daemon.sock`
- [ ] JSON 帧协议（4 字节大端长度前缀）
- [ ] `ai-config events --follow` 订阅 CLI
- [ ] `ai-config status --watch` 也走 socket
- [ ] 自启动调研（macOS launchd / Linux systemd-user / Windows Task Scheduler）— 只调研不实现
- [ ] **M4 — Phase 2 完成**

**Phase 2 exit criteria**：A-4 / A-5 / A-6 / A-9（PRD §7）验收过。

### W8 (2026-08-04 ~ 08-10) — Phase 3.a Tauri 脚手架

- [ ] `apps/ai-config-gui` crate + Tauri 2 + **React 18**（Q-2 拍板；覆盖原 "Solid" 推荐）
- [ ] 三栏布局（左：项目 / 中：资产类型 / 右：内容）
- [ ] 路由（项目 / 类型 / 条目）
- [ ] 与 daemon 通信（Tauri command 调 daemon socket）

### W9 (2026-08-11 ~ 08-17) — Phase 3.b 核心 GUI

- [ ] 项目注册 / 取消注册面板
- [ ] skills / rules 浏览 + 编辑器 + diff 预览
- [ ] agents 浏览 + 编辑
- [ ] 同步状态徽标（per-item × per-platform）
- [ ] 单条 sync 按钮

### W10 (2026-08-18 ~ 08-24) — Phase 3.c MCP 表格 + secrets + 迁移

- [ ] **MCP 表格化编辑**（一表行 = 一条 server，右侧 `${VAR}` 联动面板）
- [ ] secrets 列表（**不**显示明文值）
- [ ] `webui/data/ai-config.db` → 新 SQLite 一次性迁移器
- [ ] GUI 启动时跑一次 `doctor`，问题弹"健康检查"卡片
- [ ] **M5 — Phase 3 完成**

**Phase 3 exit criteria**：A-2 / A-10 / A-11 / A-12 / A-13 / A-14（PRD §7）验收过；非 Rust 开发者可独立使用 GUI 不查文档（人肉测试）。

### W11 (2026-08-25 ~ 08-31) — Phase 4 清理 + GA

- [ ] 删 `install.sh` / `scripts/` / `webui/`
- [ ] README 命令清单更新
- [ ] `AGENTS.md` / `HERMES.md` 命令引用同步
- [ ] 全文 grep `install.sh` / `scripts/` 残留 → 修
- [ ] v0.1 发版说明
- [ ] **M6 — GA**

**Phase 4 exit criteria**：`git grep -E '\.sh$|\.mjs$|install\.sh' -- ':!target' ':!node_modules'` 在 `ai-config/` 下 0 命中；A-2 / A-12 验收过。

### W12 ~ W22 (2026-09 ~ 2026-11) — 维护期 / v0.2 准备

- [ ] 收集用户反馈
- [ ] 修 bug / 补测试
- [ ] 准备 v0.2：
  - **i18n 英文翻译**（Q-5：v1.1 补英文）— 翻译 `zh-CN.json` → `en-US.json`
  - **OS keychain 评估**（Q-3：v1.1 评估）— `secrets` 子命令增加 `--keychain` 实验性 flag
  - **小特性**：项目模板（`templates/project/`）同步、CHANGELOG 自动化

### W23 (2026-11-30 起) — M7 Deprecate Legacy

- [ ] 老命令（旧 `install.sh` 调法）走 warning path
- [ ] 旧 webui 数据全迁完
- [ ] **M7 — Deprecate Legacy**

---

## 5. 依赖与瓶颈

### 5.1 关键依赖

- **Tauri 2 稳定版** — W8 起步，**前** W7 要在测试机上跑过 hello world
- **`notify` crate 跨平台 rename 支持** — W6 起步，macOS 已知偶发掉事件，**先**写 fixture 测试
- **`minijinja` 性能** — MCP 模板 < 1KB，重写全量没问题；如果未来模板大再换增量 diff
- **webui → 新 SQLite 迁移** — W10，依赖 W3 写好的 schema 稳定

### 5.2 风险缓冲

| 风险 | 缓冲策略 |
| --- | --- |
| Tauri 2 起步踩坑 | W8 多留 0.5 天；如超 1 天，回退到 W11 后再启 GUI |
| `notify` 平台差异 | W6 多留 1 天 fixture；3 平台都跑 |
| MCP 整文件 → 逐项迁移丢数据 | W5 留 1 天做"dry-run 预览"，人眼看一遍再 apply |
| GUI 性能 / Monaco 体积 | W9 多留 0.5 天；如超 2 天，回退 CodeMirror 6 |

### 5.3 阻塞升级

任何任务**超过估算 50%** 还没完成的 → 暂停当天其它任务 → 拆更小 → 升级到 W-2 weekly review 上讨论。

---

## 6. 资源

### 6.1 人

- **1 个开发者**（兼产品 / 测试 / 发布）— 单线程
- **AI 助手**（持续在场）— 起草代码 / 写测试 / 跑验收

### 6.2 机器

- macOS 14+（主开发 + 验收）
- Linux（Ubuntu 22.04+，周末跑一次跨平台）
- Windows 11（每月跑一次 best-effort 验收）

### 6.3 工具

- Rust 工具链（rustup + cargo + clippy + rustfmt）
- Tauri 2 CLI（`cargo install tauri-cli`）
- Node.js — **仅** webui 迁完前用，迁完后**完全卸掉**
- Docker — 干净 `$HOME` 验收容器用
- SQLite CLI — DB 调试

---

## 7. 验收节奏

| 频率 | 做什么 |
| --- | --- |
| **每日** | commit 前 `cargo test` + `cargo clippy`（CI 兜底） |
| **每周末** | 本周所有 A-N 验收过一遍；写 W-N 复盘 |
| **每个 Phase 末尾** | 全量验收（PRD §7 全部 A-N） + 回写到本 SCHEDULE.md |
| **GA 前** | 跨平台跑一遍；老用户场景过一遍 |

---

## 8. 沟通

- **每日**：无（单线程单人）
- **每周日**：W-N 复盘 + 下周计划更新（写在本文档 §4）
- **每月**：发版 / 公开进度（draft blog 或 internal note）
- **阻塞时**：当周升级到 W-N 复盘讨论

---

## 9. 变更日志

- **2026-06-12 v0.2** — 同步 8 项已决决策（PRD §12.4）；Solid→React；crate 命名 5→7 与 plan §2.2.1 对齐；补 crates.io / 预编译包 CI / brew tap / i18n 资源骨架 / 自启动集成子任务；CLI 二进制 `ai-configd` 统一为 `ai-config`；总计 41.5→46.0 人天
- **2026-06-12 v0.1** — 初版（来自 PRD v0.2 同步）
