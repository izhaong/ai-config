# 实现方案

## GUI 主题

- `getStoredThemeMode` 与 `index.html` 内联脚本默认 `system`
- `useTheme` SSR 快照同步为 `system`

## materialize

- 移除 `deploy` / `adopt_existing_deploy` 中的 `write_marker` 调用
- `deploy` 幂等：内容已 Linked 即跳过（不依赖 marker）
- 保留读取 legacy marker 的 `check` / `retract` 兼容逻辑
- 更新相关测试断言

## Todos

- [x] T001 GUI 主题默认 system（验证：theme.test + build）
- [x] T002 materialize 停止写 marker（验证：cargo test -p ai-config-core）
- [x] T003 删除仓内已有 `.ai-config-deploy.json`（验证：grep 无新增）
