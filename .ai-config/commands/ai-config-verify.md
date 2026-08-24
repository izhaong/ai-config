---
description: 运行 agent-manager 标准验证（test、build、doctor）
---

## User Input

```text
$ARGUMENTS
```

## 说明

执行 skill **agent-manager-verify**（`.agent-manager/skills/agent-manager-verify/SKILL.md`）。

## 默认命令集

在 **agent-manager 仓库根**：

```bash
cargo test -p agent-manager-core -p agent-manager-cli
cargo build -p agent-manager-cli
./target/debug/agent-manager doctor
```

若 `$ARGUMENTS` 含 `gui` 或近期改了 `apps/agent-manager-gui`：

```bash
cd apps/agent-manager-gui && npm run build
```

若 `$ARGUMENTS` 含 `workspace`：

```bash
cargo test --workspace
```

## 输出

- 各命令退出码与摘要
- 失败时给出最短修复建议
- 全部通过才声明「验证通过」
