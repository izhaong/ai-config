# ai-config — Agent 入口（Codex / Cursor / Claude Code）

多 IDE 共享的 **配置管理工具**仓库。用户 skills/rules/commands/MCP 在 **`~/.ai-config/`**，不在本仓。

## 加载顺序

1. 本文件 `AGENTS.md`
2. `.cursor/rules/00-ai-config-core.mdc`（核心工作流）
3. `.cursor/rules/spec-kit-gate.mdc`（非平凡改动先 spec）
4. `.cursor/rules/karpathy-guidelines.mdc`
5. 按任务读 `.cursor/agents/*.md` 与 `.cursor/skills/*/SKILL.md`
6. Hermes 在仓内改码：另读 `HERMES.md`

## 仓库地图

| 路径                      | 说明                                                            |
| ------------------------- | --------------------------------------------------------------- |
| `crates/ai-config-core`   | **唯一业务逻辑**                                                |
| `crates/ai-config-cli`    | CLI（`ai-config` 二进制）                                       |
| `crates/ai-config-daemon` | 守护进程                                                        |
| `apps/ai-config-gui`      | Tauri 2 + React GUI（ahooks / es-toolkit / shadcn 见 gui 规则） |
| `docs/product/`           | PRD / 架构 / 设计                                               |
| `specs/`                  | Spec Kit 功能规格                                               |
| `.specify/`               | constitution、模板、脚本                                        |
| `.cursor/`                | rules、agents、skills、commands                                 |

## Agent 路由

| 任务                     | Agent                                  |
| ------------------------ | -------------------------------------- |
| Rust core / CLI / daemon | `.cursor/agents/rust-ai-config-dev.md` |
| GUI React / Tauri 对接   | `.cursor/agents/gui-ai-config-dev.md`  |

## Skills

| Skill                 | 用途                           |
| --------------------- | ------------------------------ |
| `ai-config-delivery`  | Spec → 分支 → 实现 → CHANGELOG |
| `ai-config-verify`    | test / build / doctor          |
| `karpathy-guidelines` | 编码行为准则                   |

Commands 索引：`.cursor/COMMANDS.md`（`/speckit.*`、`/ai-config-verify`、`/ai-config-sync`）

## Spec Kit

非平凡功能：**spec → plan → tasks → implement**（宪法：`.specify/memory/constitution.md`）。

若分支为 `feat/*`（Git Flow）而非 `001-name`，运行 spec 脚本前：

```bash
export SPECIFY_FEATURE=001-your-feature   # 对应 specs/ 子目录名
```

## 快速构建

```bash
cargo build -p ai-config-cli --release
./target/release/ai-config install
```

## 验证（完成前）

```bash
cargo test -p ai-config-core -p ai-config-cli
cd apps/ai-config-gui && npm run build
ai-config doctor
```

## 文档

- [README.md](./README.md)
- [docs/README.md](./docs/README.md)
- [CONTRIBUTING.md](./CONTRIBUTING.md)

## Git

- 集成线 `develop`；功能分支 `feat/*`、`fix/*`
- 中文 Conventional Commits；未要求时不主动 commit/push
