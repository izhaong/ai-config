# agent-manager Cursor Commands

**用户可直接用自然语言下发任务**；Agent 按 rules/skills 自动走分支与 spec，**不依赖**下列命令。

下列命令供 **Agent 步骤参考** 或高级用户手动触发：

| 命令                 | 说明（Agent 通常内化，勿要求用户输入）           |
| -------------------- | ------------------------------------------------ |
| `/speckit.specify`   | 创建 `spec.md`                                   |
| `/speckit.plan`      | 写 `plan.md`（含 `## Todos` = Cursor Plan 清单） |
| `/speckit.implement` | 按 plan `## Todos` 实现并验证                    |
| `/agent-manager-verify`  | cargo test、GUI build、doctor                    |
| `/agent-manager-sync`    | 本地安装/同步冒烟与排障指引                      |

Spec Kit 宪法：`.specify/memory/constitution.md`
