# ai-config 文档

## 产品与设计（桌面端工具）

| 文档                                         | 说明                                                      |
| -------------------------------------------- | --------------------------------------------------------- |
| [PRD.md](./product/PRD.md)                   | 历史产品需求 v0.5（硬拷贝语义已被 Spec 008 取代，仅供迁移盘点） |
| [ARCHITECTURE.md](./product/ARCHITECTURE.md) | 技术架构：crate 边界、materialize、数据流                 |
| [DESIGN.md](./product/DESIGN.md)             | GUI 信息架构 + §19 已实现交互附录                         |
| [SCHEDULE.md](./product/SCHEDULE.md)         | 里程碑与排期                                              |

## 参考资料

| 文档                                                                                 | 说明                                                                                                       |
| ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| [platform-contracts.md](./reference/platform-contracts.md)                           | **当前四平台资产契约索引**；唯一权威需求为 Spec 008                                                       |
| [claudemarketplace-skills-top100.md](./reference/claudemarketplace-skills-top100.md) | [Claude Code Marketplace](https://www.claudemarketplace.net/skills) 下载量 / Stars Top 100                 |
| [vercel-skills-agent-paths.md](./reference/vercel-skills-agent-paths.md)             | 第三方 CLI 历史快照，只作 legacy inventory，不能决定 ai-config 投影路径                                   |
| [hooks-compatibility-matrix.md](./reference/hooks-compatibility-matrix.md)           | Hooks 在 Cursor / Claude / Codex 的事件映射、路径和 matcher 兼容策略                                       |

## 资产与配置

| 文档                                          | 说明                                                   |
| --------------------------------------------- | ------------------------------------------------------ |
| 用户资产 `~/.ai-config/`                      | 全局 skills / rules / mcp / agents / commands / prompts / hooks（本机，不在仓库内） |
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
