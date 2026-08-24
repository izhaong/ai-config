---
description: 基于 spec.md 生成 plan.md（详细技术方案 + ## Todos，贴合 Cursor Plan）
handoffs:
  - label: Implement
    agent: speckit.implement
    prompt: 按 plan.md ## Todos 实现
---

## User Input

```text
$ARGUMENTS
```

## 上下文

- 模板：`.specify/templates/plan-template.md`
- 架构：`docs/product/ARCHITECTURE.md`
- 输入：`specs/<feature>/spec.md`
- 输出：`specs/<feature>/plan.md`（**含 `## Todos`**）

## 步骤

1. 解析 `FEATURE_DIR`（`check-prerequisites.sh --json` 或分支/specs 推断）。
2. 阅读 `spec.md`、`constitution.md`；**读相关源码**（grep / 打开 FR 涉及的模块），勿凭空写方案。
3. 编写 **plan.md**（**尽量详细**，参考 `specs/005-sync-hooks-tts/plan.md` 与扩写后的 `006-install-project-merge/plan.md`）：

### 必填章节（缺一视为 plan 未完成）

| 章节                   | 最低要求                                          |
| ---------------------- | ------------------------------------------------- |
| **Summary**            | 目标 + 技术路线 + 明确非目标                      |
| **背景与根因**         | 复现步骤；**文件/函数级**根因；可选调用链图       |
| **Technical Context**  | crate、模块、测试命令、涉及平台                   |
| **Constitution Check** | 勾选式自检                                        |
| **影响面**             | 文件表 + 新增/变更 API 签名 + **FR→实现**追溯表   |
| **方案设计**           | 分小节写清算法与交互；有全局/项目双模式时须对比表 |
| **边界与风险**         | 表格：场景 / 期望 / 计划测试名                    |
| **测试策略**           | 列举具体 test 与断言要点                          |
| **回滚**               | revert 与数据影响                                 |
| **`## Todos`**         | 可独立交付；每项含 `（验证：…）`                  |

### Todos 粒度

- 顺序：**core → CLI → Tauri → GUI → CHANGELOG → verify**
- 一项 Todo ≈ 一个 PR 内可 review 的单元（避免「改 hook」这种笼统项）
- 末项固定：`cargo test -p agent-manager-core -p agent-manager-cli`（及 GUI/doctor 若涉及）
- 格式：`- [ ] T00n [scope] 动词 + 对象（验证：命令或 test 名）`

4. Todos 即 Cursor Plan 的执行清单；**不生成独立 `tasks.md`**。
5. 若 Agent 使用 TodoWrite，将 `## Todos` 同步到会话 Todo。

## 约束

- 业务逻辑落在 `agent-manager-core`
- Karpathy：最简单可行路径；plan 可写备选方案与取舍，但实现只选一条
- **禁止**仅 3 行根因 + 模块表 + 5 条笼统 Todo 交差
