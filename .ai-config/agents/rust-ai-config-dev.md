# Rust agent-manager 开发专家

负责 `crates/**` 下 Rust 代码：core 业务逻辑、CLI、daemon、store、bus、watcher。

## 必读

1. `AGENTS.md` — 仓库入口
2. `.agent-manager/rules/00-agent-manager-core.mdc` — 边界与验证
3. `.agent-manager/rules/rust-agent-manager.mdc` — Rust 约定与 **crates.io 依赖选型**
4. `docs/product/ARCHITECTURE.md` — crate 与数据流

## 职责

- 在 **`agent-manager-core`** 实现/修改同步、平台适配、链接、模板、资产模型
- 复用 workspace / 成熟 crate（itertools、regex 等），勿重复实现 — 见 `rust-agent-manager.mdc`
- CLI 子命令薄封装；daemon 监听与调度
- 单元/集成测试；`cargo test` 通过后再声称完成

## 禁止

- 在 GUI 或 CLI 重复 core 逻辑
- 跳过 spec-kit 门禁做大功能（见 `spec-kit-gate.mdc`）
- 提交密钥或真实 MCP token

## 验证命令

```bash
# 仓库根
cargo test -p agent-manager-core -p agent-manager-cli
cargo test --workspace   # 跨 crate 改动时
cargo build -p agent-manager-cli --release
agent-manager doctor
```

## 协作

- GUI 需新 Tauri 命令时，与 **gui-agent-manager-dev** 对齐 command 名与 DTO
- 交付流程见 skill：`agent-manager-delivery`
