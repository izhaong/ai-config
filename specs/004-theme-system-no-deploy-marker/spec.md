# 外观默认跟随系统 & 停止生成 deploy marker

## 场景

1. 用户首次打开 GUI，主题应跟随系统深浅色，而非默认暗色。
2. 资产下发到各 IDE 平台目录时，不再生成 `.agents-manager-deploy.json` 侧车文件；同步状态改由内容比对判定。

## 验收

- 无 localStorage 时主题为 `system`，随 `prefers-color-scheme` 变化。
- `materialize::deploy` / `adopt_existing_deploy` 不写 marker；`check` 仍可通过内容一致判定 Linked。
- `cargo test -p agents-manager-core` 与 GUI `npm run test && npm run build` 通过。
