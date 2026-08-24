# 实现方案

## 模块

| 路径 | 职责 |
| ---- | ---- |
| `core/workspace.rs` | `discover_members`、`effective_asset_root`、`resolve_member_sync_roots` |
| `core/sync.rs` | `compute_for_sync_roots` |
| `cli/lifecycle.rs` | `run_install_workspace`；`--workspace` sync |
| `cli/main.rs` | `--workspace` 全局 flag |

## 资产继承

```
member/.agents-manager 有内容? → 用 member
否则 workspace/.agents-manager 存在? → 用 workspace
否则 scan 仅 global (~/.agents-manager)
```

## Todos

- [x] T001 `workspace.rs` + 单测
- [x] T002 `compute_for_sync_roots` + `load_context_from_roots`
- [x] T003 CLI `--workspace` on install/sync + JSON 报告
- [x] T004 lifecycle workspace 集成测
- [x] T005 `cargo test -p agents-manager-core -p agents-manager-cli`
