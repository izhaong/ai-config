# 实现方案

## 根因

1. `load_context` 把 `AI_CONFIG_ROOT=<repo>` 当作资产根 `scan_project_root`，未合并 `~/.agent-manager`
2. `entry_matches_token` 用 `/hooks/foo` 子串匹配，误删 `.cursor/hooks/foo` 用户条目
3. 项目 install 对 commands/skills 等直接 `materialize::deploy` 覆盖已有文件

## 改动

| 模块 | 内容 |
| ---- | ---- |
| `paths.rs` | `resolve_sync_roots`、`is_project_deploy_base` |
| `lifecycle.rs` | `load_context` 使用新解析；Create 项目作用域跳过非托管覆盖 |
| `hook_adapter.rs` | 收紧 token 匹配；Claude 共享 hooks 目录；保留 `mcpServers` |

## Todos

- [x] T001 `resolve_sync_roots` + 单测（验证：paths tests）
- [x] T002 `load_context` / install 作用域修复（验证：lifecycle test）
- [x] T003 hook merge 仅移除托管条目 + mcpServers 保留 + Claude 共享路径（验证：hook_adapter tests）
- [x] T004 项目 install 跳过非托管 Create 覆盖（验证：materialize/lifecycle）
- [x] T005 `cargo test -p agent-manager-core -p agent-manager-cli`
