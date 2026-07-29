---
name: ai-config-verify
description: ai-config 四层验收：源码测试、隔离生命周期、已安装运行态、人工外部门禁。声称完成前必跑。
---

# ai-config 验证

验证必须区分四层结果，不能用源码测试替代已安装版本或真实 IDE 验收。

## 一键验证

```bash
.ai-config/skills/ai-config-verify/scripts/verify-closure.sh all
```

可按层运行：

```bash
.ai-config/skills/ai-config-verify/scripts/verify-closure.sh source
.ai-config/skills/ai-config-verify/scripts/verify-closure.sh sandbox
.ai-config/skills/ai-config-verify/scripts/verify-closure.sh runtime
```

## 1. Source：源码自动化

- `cargo test --workspace`
- `cargo build -p ai-config-cli`
- `npm run test && npm run build`
- 行为变更必须包含本次新增/更新的回归用例。

快速聚焦 core/CLI 时可先跑：

```bash
cargo test -p ai-config-core -p ai-config-cli
```

## 2. Sandbox：临时 HOME 生命周期

隔离脚本必须使用 `mktemp -d` 的 HOME 与资产根，覆盖：

- sync 默认仅生成 plan，平台目标零写入；
- `sync --apply` 创建受管目标；
- status 无 broken / wrong source / wrong type；
- uninstall 默认仅生成 plan；
- `uninstall --apply` 收回受管目标并保留 canonical source；
- doctor、completion 完整消费和提前关闭。

禁止为冒烟覆盖真实 HOME。禁止调用旧版 `sync` 猜测 `--dry-run` 语义。

## 3. Runtime：已安装运行态

先比较：

- `target/debug/ai-config --version`
- `command -v ai-config` 与 `ai-config --version`

版本不一致立即停止，不能调用旧二进制的 sync。版本一致后，真实环境只读执行：

- `doctor --json`
- `status --json`
- `secrets validate --json`

Runtime 通过要求：doctor 无阻塞、无 literal MCP secret；status 无 broken / wrong_source / wrong_type；secrets 无缺失。

## 4. Manual：人工/外部门禁

以下不由自动化结果代替：

- GUI 关键交互与视觉检查；
- Cursor / Codex / Claude / Hermes 重启后资产可见性；
- daemon 长驻、文件变更触发和系统重启恢复；
- 真实 provider/MCP 连通性；
- 发布包、自动更新、签名与目标操作系统验收。

## 结论格式

交付报告逐层写 `PASS` / `FAIL` / `NOT RUN`，并列出：

- 构建版本与安装版本；
- sandbox 生命周期证据；
- runtime drift/blocker；
- manual 外部门禁。

未通过不得标 Todo/Plan 为 completed。
