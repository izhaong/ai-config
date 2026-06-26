---
name: ai-config-verify
description: ai-config 变更验证：cargo test、GUI build、ai-config doctor/status。声称完成前必跑。
---

# ai-config 验证

## 快速（core / CLI 改动）

在仓库根：

```bash
cargo test -p ai-config-core -p ai-config-cli
cargo build -p ai-config-cli
./target/debug/ai-config doctor
./target/debug/ai-config status
```

## 全 workspace（daemon / store / watcher）

```bash
cargo test --workspace
```

## GUI

```bash
cd apps/ai-config-gui
npm run test
npm run build
```

有 Tauri command 改动：

```bash
cargo test -p ai-config-gui
```

## 新功能测试门禁

- **Rust**：`ai-config-core` / CLI 行为 → 模块内 `#[test]` 或 `tests/`，与实现同批提交。
- **GUI**：`utils` / `hooks` / 关键组件 → Vitest（`*.test.ts(x)`），覆盖主路径与关键分支。
- 只改样式/文案且无语义变更可例外；**交互、状态机、Tauri 契约变更必须补测**。
- 声称完成前：`cargo test -p ai-config-core -p ai-config-cli` 与 `npm run test` 均 0 失败。

## 同步冒烟（改了 platform / sync / command）

```bash
ai-config command list
ai-config skill list    # 若有本地 ~/.ai-config 资产
ai-config sync --dry-run   # 若支持
```

## 通过标准

- 测试 0 失败（含 **本次改动新增/更新的用例**）
- `npm run test && npm run build` 无 TS 错误
- `doctor` 无阻塞项（按当前产品定义）

未通过不得标 Todo/Plan 为 completed。
