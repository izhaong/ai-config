# Implementation Plan: 验证闭环强化

**Branch**: `fix/verify-e2e-closure` | **Date**: 2026-07-29 | **Spec**: [spec.md](./spec.md)

## Summary

本变更把 `agents-manager-verify` 从“测试命令清单”升级为四层验收：源码自动化、临时 HOME 隔离生命周期、已安装运行态版本/只读状态、人工外部门禁。同时修复 completion stdout 提前关闭时的 panic，并补可回归测试。

不修改 projection/core 产品语义，不在真实 HOME 执行 0.4.0 `sync --apply`，不自动迁移用户 MCP/Hook 资产。

## 背景与根因

`crates/agents-manager-cli/src/main.rs` 的 `Cmd::Completion` 直接把 `Stdout` 交给 `clap_complete::generate`。该依赖在 writer 返回 BrokenPipe 时内部 panic，因此常见的 `completion zsh | head` 以 101 退出。

`.agents-manager/skills/agents-manager-verify/SKILL.md` 目前只列出 cargo/npm/doctor/status 命令：

- 未比较 `target/debug/agents-manager` 与 PATH 中的真实安装版本；
- 建议的 `agents-manager sync --dry-run` 与当前 0.4.0 `--apply` 契约不一致，旧版甚至可能默认写入；
- 没有临时 HOME 的 apply/retract 生命周期；
- 没有区分自动化、运行态与人工验收边界。

```text
source tests ──► build binary ──► isolated HOME lifecycle
                                      │
                                      ▼
installed version guard ──► real read-only doctor/status
                                      │
                                      ▼
                               manual external gates
```

## Technical Context

| 项         | 值 |
| ---------- | -- |
| Language   | Rust 2021 + Bash |
| 主要 Crate | `agents-manager-cli` |
| 依赖模块   | `clap_complete`、CLI output boundary |
| 测试       | `cargo test -p agents-manager-cli`；verify shell E2E |
| 平台矩阵   | Cursor / Codex / Claude / Hermes（隔离全局 scope） |

## Constitution Check

- [x] completion 属 CLI 输出边界，不新增 core 业务逻辑
- [x] CLI/GUI 不复制 projection 逻辑
- [x] 不改变 platform adapter
- [x] 隔离 E2E 验证幂等和显式 apply
- [x] 验证项对齐 constitution §5

## 影响面

### Crate / 文件

| Crate/区域 | 文件 | 变更类型 | 说明 |
| ---------- | ---- | -------- | ---- |
| `agents-manager-cli` | `src/main.rs` | 修改 | completion 先渲染到内存，再安全写 stdout |
| `agents-manager-cli` | `src/main.rs` tests | 新增 | BrokenPipe 与普通 IO 错误回归 |
| project skill | `.agents-manager/skills/agents-manager-verify/SKILL.md` | 修改 | 四层验证门禁 |
| project skill | `.agents-manager/skills/agents-manager-verify/scripts/verify-closure.sh` | 新增 | 版本检查与隔离 E2E |
| docs | `CHANGELOG.md` | 修改 | 记录 CLI 修复与验证增强 |

### API / 类型

不新增公开 core API。CLI 内新增 completion writer helper，返回 `std::io::Result<()>`。

### 需求追溯

| FR | 实现位置 | 验收方式 |
| -- | -------- | -------- |
| FR-001 | `verify-closure.sh runtime` | 构造不同版本命令/当前安装版本门禁 |
| FR-002/003 | `verify-closure.sh sandbox` | 临时 HOME lifecycle |
| FR-004/005 | CLI completion helper | Rust 单测 + 真实管道 |
| FR-006/007 | `SKILL.md` | 文档审查 + doctor/status 摘要 |

## 方案设计

### Completion 输出

先让 `clap_complete` 写入 `Vec<u8>`；内存 writer 不会产生 BrokenPipe。再使用显式 `write_all` 写 stdout：

- `Ok` → 0；
- `ErrorKind::BrokenPipe` → 0，不输出错误；
- 其它 IO 错误 → stderr 简洁信息，退出码 5。

### 隔离验证脚本

脚本分为：

- `source`：构建与自动化测试；
- `sandbox`：创建受控临时 HOME/资产，执行 plan → apply → status → uninstall apply；
- `runtime`：比较安装路径/版本，并只读执行 doctor/status；
- `all`：顺序执行前三层。

临时目录使用 `mktemp -d`，trap 只删除已校验位于系统临时根下的本次目录。所有写命令必须显式传 `--root`、临时 `HOME` 和 `--apply`。

### 运行态门禁

运行态检查首先比较版本。版本不一致时停止，不调用旧二进制的 sync。版本一致后也只执行 doctor/status/secrets validate；真实 apply 仍需独立用户决策。

## 边界与风险

| 场景 | 期望行为 | 测试名/验证 |
| ---- | -------- | ----------- |
| stdout BrokenPipe | 安静成功 | `completion_broken_pipe_is_success` |
| stdout Other IO error | 退出 5 | `completion_other_io_error_fails` |
| sync plan 有 blocker | sandbox 停止且零 apply | 脚本 jq 断言 |
| 安装版本落后 | runtime 失败并报告 | 版本门禁 |
| sandbox 中断 | 清理临时目录 | trap |

**非目标 / 不做**：修复现有用户 MCP source-first 迁移、自动处理所有 unsupported platform contract、GUI 视觉自动化。

## 测试策略

1. CLI 单元测试覆盖 completion writer 两个错误分支。
2. CLI 真实管道验证 `completion zsh | head`。
3. 隔离 shell E2E 覆盖同步生命周期。
4. 回归运行 `cargo test --workspace`。
5. GUI 运行 `npm run test && npm run build`。
6. 运行态只读检查 doctor/status/secrets；确认安装版本。

## 回滚

代码与 skill 均可通过单次 revert 回滚。隔离 E2E 不保留数据。若升级本机二进制，可重新安装上一发布版本；不回滚或覆盖用户真实平台配置。

## Todos

- [x] T001 为 completion BrokenPipe 与普通 IO 错误补回归测试（验证：`cargo test -p agents-manager-cli completion_`）
- [x] T002 实现 completion 安全输出并通过真实管道验证（验证：`set -o pipefail; ./target/debug/agents-manager completion zsh | head -n 2`）
- [x] T003 新增版本门禁与临时 HOME 生命周期脚本（验证：`verify-closure.sh sandbox`）
- [x] T004 更新 `agents-manager-verify` 四层验收说明与 CHANGELOG（验证：人工检查命令无真实 HOME 写入）
- [ ] T005 运行增强后的 agents-manager-verify 全量验证（验证：workspace、GUI、sandbox、runtime）

## Verification Status

- **Source — PASS**：Rust workspace 647 tests；GUI 20 files / 92 tests；GUI production build 成功。
- **Sandbox — PASS**：plan 零写入、sync apply、status、uninstall plan/apply、source 保留、completion 提前关闭均通过。
- **Runtime — FAIL**：构建与安装版本已一致为 0.4.0；真实配置仍有 26 个 literal MCP secret、3 个遗留 secret 文件和 2 个 MCP wrong_source。
- **Manual — NOT RUN**：GUI 视觉、四 IDE 重启可见性、daemon 长驻、真实 MCP provider 连通性仍需独立验收。
