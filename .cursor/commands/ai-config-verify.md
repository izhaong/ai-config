---
description: 运行 ai-config 标准验证（test、build、doctor）
---

## User Input

```text
$ARGUMENTS
```

## 说明

执行 skill **ai-config-verify**（`.cursor/skills/ai-config-verify/SKILL.md`）。

## 默认命令集

在 **ai-config 仓库根**：

```bash
cargo test -p ai-config-core -p ai-config-cli
cargo build -p ai-config-cli
./target/debug/ai-config doctor
```

若 `$ARGUMENTS` 含 `gui` 或近期改了 `apps/ai-config-gui`：

```bash
cd apps/ai-config-gui && npm run build
```

若 `$ARGUMENTS` 含 `workspace`：

```bash
cargo test --workspace
```

## 输出

- 各命令退出码与摘要
- 失败时给出最短修复建议
- 全部通过才声明「验证通过」
