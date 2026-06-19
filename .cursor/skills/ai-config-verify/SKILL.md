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
npm run build
```

有 Tauri command 改动：

```bash
cargo test -p ai-config-gui
```

## 同步冒烟（改了 platform / sync / command）

```bash
ai-config command list
ai-config skill list    # 若有本地 ~/.ai-config 资产
ai-config sync --dry-run   # 若支持
```

## 通过标准

- 测试 0 失败
- `npm run build` 无 TS 错误
- `doctor` 无阻塞项（按当前产品定义）

未通过不得标 Todo/Plan 为 completed。
