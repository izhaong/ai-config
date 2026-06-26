# 实现方案

## 源资产形态

```
.ai-config/hooks/<name>/
  HOOK.md          # 人类可读（description，可选）
  hook.yaml        # canonical
  scripts/         # 或 hook.yaml.script_dir 指向其它子目录
```

`hook.yaml` 草案：

```yaml
name: lifecycle-tts
description: ai-config 生命周期 TTS 播报
script_dir: scripts
entry: lifecycle-tts.sh
events:
  - after_shell          # canonical 事件名
    matcher: 'ai-config\b'
enabled_platforms: [cursor, codex, claude, hermes]  # 可选，默认四端凡 supports 者
```

## Core 模块

| 模块                              | 职责                                   |
| --------------------------------- | -------------------------------------- |
| `source::scan_hooks`              | `hooks/<name>/hook.yaml`               |
| `hook_adapter`                    | canonical → 平台片段；merge / retract  |
| `asset_ops::deploy/retract(Hook)` | 与 skill 同入口，内部调 adapter        |
| `platform::supports(Hook)`        | 四端均为 true（Hermes 走 config.yaml） |

## 平台路径（脚本落地）

| 平台   | 脚本目录                        | 配置合并目标                        |
| ------ | ------------------------------- | ----------------------------------- |
| cursor | `~/.cursor/hooks/<name>/`       | `~/.cursor/hooks.json`              |
| codex  | `~/.codex/hooks/<name>/`        | `~/.codex/hooks.json`               |
| claude | `~/.claude/hooks/<name>/`       | `~/.claude/settings.json` → `hooks` |
| hermes | `~/.hermes/agent-hooks/<name>/` | `~/.hermes/config.yaml` → `hooks`   |

## 迁移

- `hooks/hooks.json` + 根级 `.sh` → `hooks/lifecycle-tts/`
- 删除 v0 整包 `hook_deploy::deploy_hooks` 批量路径，改 per-item

## Todos

- [x] T000 v0 整包 hook_deploy + TTS 脚本（过渡）
- [x] T001 `hook.yaml` schema + `scan_hooks` + `AssetKind::Hook`
- [x] T002 `hook_adapter`（cursor/codex/claude/hermes merge+retract）+ 单测
- [x] T003 `asset_ops` + lifecycle/sync 接入 per-hook
- [x] T004 迁移 `lifecycle-tts` 目录资产
- [x] T005 GUI 第六类 + i18n + 平台 icon
- [x] T006 verify：`cargo test` + GUI build
