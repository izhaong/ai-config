---
description: 运行 agents-manager 标准验证（test、build、doctor）
---

## User Input

```text
$ARGUMENTS
```

## 说明

执行 skill **agents-manager-verify**（`.agents-manager/skills/agents-manager-verify/SKILL.md`）。

## 默认命令集

在 **agents-manager 仓库根**：

```bash
cargo test -p agents-manager-core -p agents-manager-cli
cargo build -p agents-manager-cli
./target/debug/agents-manager doctor
```

若 `$ARGUMENTS` 含 `gui` 或近期改了 `apps/agents-manager-gui`：

```bash
cd apps/agents-manager-gui && npm run build
```

若 `$ARGUMENTS` 含 `workspace`：

```bash
cargo test --workspace
```

## 输出

- 各命令退出码与摘要
- 失败时给出最短修复建议
- 全部通过才声明「验证通过」
