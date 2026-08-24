# agents-manager — Agent 入口（Codex / Cursor / Claude Code）

多 IDE 共享的 **配置管理工具**仓库。用户 skills/rules/commands/MCP 在 **`~/.agents-manager/`**，不在本仓。

## 加载顺序

1. 本文件 `AGENTS.md`
2. `.agents-manager/rules/00-agents-manager-core.mdc`（核心工作流）
3. `.agents-manager/rules/spec-kit-gate.mdc`（非平凡改动先 spec）
4. `.agents-manager/rules/karpathy-guidelines.mdc`
5. 按任务读 `.agents-manager/agents/*.md` 与 `.agents-manager/skills/*/SKILL.md`
6. 编写 Hook 脚本 / `hooks.json` → `.agents-manager/rules/hooks-asset-layout.mdc`
7. Hermes 在仓内改码：另读 `HERMES.md`

## 仓库地图

| 路径                      | 说明                                                            |
| ------------------------- | --------------------------------------------------------------- |
| `crates/agents-manager-core`   | **唯一业务逻辑**                                                |
| `crates/agents-manager-cli`    | CLI（`agents-manager` 二进制）                                       |
| `crates/agents-manager-daemon` | 守护进程                                                        |
| `apps/agents-manager-gui`      | Tauri 2 + React GUI（ahooks / es-toolkit / shadcn 见 gui 规则） |
| `docs/product/`           | PRD / 架构 / 设计                                               |
| `specs/`                  | Spec Kit 功能规格                                               |
| `.specify/`               | constitution、模板、脚本                                        |
| `.agents-manager/`             | 唯一项目 source：rules、agents、skills、commands、prompts、hooks |

## Agent 路由

| 任务                     | Agent                                  |
| ------------------------ | -------------------------------------- |
| Rust core / CLI / daemon | `.agents-manager/agents/rust-agents-manager-dev.md` |
| GUI React / Tauri 对接   | `.agents-manager/agents/gui-agents-manager-dev.md`  |
| 测试 / 验收 / 验证闭环   | `.agents-manager/agents/qa-agents-manager-dev.md`   |

## Skills

| Skill                 | 用途                           |
| --------------------- | ------------------------------ |
| `agents-manager-delivery`  | Spec → 分支 → 实现 → CHANGELOG |
| `agents-manager-verify`    | test / build / doctor          |
| `karpathy-guidelines` | 编码行为准则                   |

Commands 索引：`.agents-manager/commands/`（`/speckit.*`、`/agents-manager-verify`、`/agents-manager-sync`）

## Spec Kit（用户零负担）

用户用自然语言下发任务即可。Agent **自动**完成：分支 → `specs/<feature>/`（`spec.md` → `plan.md` 含 Todos）→ 按 Todos 实现 → 验证。

- 宪法：`.specify/memory/constitution.md`
- 门禁与自动解析：`.agents-manager/rules/spec-kit-gate.mdc`、`.agents-manager/rules/gitflow-spec-kit.mdc`
- 交付编排：skill `agents-manager-delivery`（内化 `/speckit.*`，**不要求用户输入这些命令**）
- `SPECIFY_FEATURE`、切分支、写 spec：均由 Agent 在会话内自行处理

## 快速构建

```bash
cargo build -p agents-manager-cli --release
./target/release/agents-manager install
```

## 验证（完成前）

```bash
cargo test -p agents-manager-core -p agents-manager-cli
cd apps/agents-manager-gui && npm run build
agents-manager doctor
```

## 文档

- [README.md](./README.md)
- [docs/README.md](./docs/README.md)
- [CONTRIBUTING.md](./CONTRIBUTING.md)

## Git

- 集成线 `develop`；功能分支 `feat/*`、`fix/*`
- 中文 Conventional Commits；未要求时不主动 commit/push
