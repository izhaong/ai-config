# ai-config Constitution

> Spec Kit 项目原则。非平凡改动须先 `specs/<编号-功能>/spec.md` → `plan.md`（含 `## Todos`），再实现。

## 1. 产品边界

- **做什么**：跨 IDE（Cursor / Codex / Claude / Hermes）统一安装、同步、管理 skills、rules、commands、MCP、agents。
- **不做什么**：不在本仓存放用户真实资产与密钥；业务项目代码不在本仓实现。
- **权威产品文档**：`docs/product/PRD.md`、`ARCHITECTURE.md`、`DESIGN.md`。

## 2. 架构铁律

1. **`ai-config-core` 是唯一业务逻辑归宿** — CLI、daemon、GUI 只调用 core，禁止在 `apps/` 或 `cli` 重复实现同步/链接/模板逻辑。
2. **平台差异收敛在 `platform` 模块** — 新增 IDE 支持 = 新 adapter + 测试，不 scattered if-else。
3. **幂等与可回滚** — 链接/覆盖前备份；`sync` / `install` 可重复执行。
4. **密钥隔离** — 真实 token 只进 `~/.config/ai-config/secrets.env`（0600）；仓库内仅 `.example` 占位符。

## 3. 实现准则（Karpathy）

1. **先想清楚再写** — 多解法则说明取舍；不确定则问。
2. **够用即可** — 不做未请求的功能、单用抽象、过度错误分支。
3. **手术式修改** — 只动任务相关文件；风格与周边一致。
4. **可验证完成** — 声称完成前须跑通约定验证（见 §5）。

## 4. Git 与交付

- 集成线 **`develop`**，稳定线 **`main`**；禁止直推主干。
- 功能从 `develop` 拉 `feat/<issue>-<简述>` 或 `fix/...`，小步中文 Conventional Commits。
- PR 合并后：`fetch` → `checkout develop` → `pull --ff-only`。

## 5. 验证门禁

| 范围                   | 最低验证                                            |
| ---------------------- | --------------------------------------------------- |
| `ai-config-core` / CLI | `cargo test -p ai-config-core -p ai-config-cli`     |
| 全 workspace Rust      | `cargo test --workspace`（改 daemon/store 时）      |
| GUI 前端               | `cd apps/ai-config-gui && npm run build`            |
| GUI + Tauri 命令       | `cargo test -p ai-config-gui`（有 Rust 命令改动时） |
| 同步行为               | `ai-config doctor` + 目标平台 `status`              |

未跑验证不得将 Plan/Todo 标为 completed。

## 6. 文档同步

代码行为或 CLI 子命令变化时，同一 PR 更新：`CHANGELOG.md`、`AGENTS.md`（路由变化时）、`docs/product/`（产品语义变化时）。

## 7. Spec Kit 工作流（贴合 Cursor）

用户自然语言下发任务。**Spec Kit 适配 Cursor Plan/Todo**，不另造平行流程：

```
specs/N-feature/spec.md → plan.md（含 ## Todos）→ 按 Todos 实现 → §5 验证
```

`plan.md` 的 Todos = Cursor Plan 的执行清单；**不维护 `tasks.md`**。

**例外**（可跳过完整 spec 周期）：单行 typo、明显 bug 一行修复、纯格式化、用户明示 spike。
