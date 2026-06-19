# ai-config 文档

## 产品与设计（桌面端工具）

| 文档                                         | 说明                                   |
| -------------------------------------------- | -------------------------------------- |
| [PRD.md](./product/PRD.md)                   | 产品需求：做什么、资产模型、验收标准   |
| [ARCHITECTURE.md](./product/ARCHITECTURE.md) | 技术架构：crate 边界、数据流、关键决策 |
| [DESIGN.md](./product/DESIGN.md)             | GUI 信息架构：三栏、状态、交互线框     |
| [SCHEDULE.md](./product/SCHEDULE.md)         | 里程碑与排期                           |

## 资产与配置

| 文档                                          | 说明                                                   |
| --------------------------------------------- | ------------------------------------------------------ |
| 用户资产 `~/.ai-config/`                      | 全局 skills / rules / mcp / agents（本机，不在仓库内） |
| [manifests/README.md](../manifests/README.md) | 插件 skill 索引（不入库）                              |

## AI 协作（本仓工具开发）

| 路径                                  | 说明                                                 |
| ------------------------------------- | ---------------------------------------------------- |
| [AGENTS.md](../AGENTS.md)             | Codex / Cursor / Claude 统一入口                     |
| [CLAUDE.md](../CLAUDE.md)             | Claude Code 别名入口                                 |
| [HERMES.md](../HERMES.md)             | Hermes 加载说明                                      |
| `.cursor/rules/`                      | 项目规则（core 边界、Rust、GUI、Spec Kit、Karpathy） |
| `.cursor/agents/`                     | `rust-ai-config-dev`、`gui-ai-config-dev`            |
| `.cursor/skills/`                     | 交付、验证、行为准则                                 |
| `.cursor/COMMANDS.md`                 | `/speckit.*`、`/ai-config-verify` 等                 |
| `.specify/memory/constitution.md`     | Spec Kit 项目宪法                                    |
| [specs/README.md](../specs/README.md) | 功能规格目录约定                                     |

## 入口（仓库根）

| 文件                      | 说明         |
| ------------------------- | ------------ |
| [README.md](../README.md) | 人类快速上手 |
