# ai-config — 多 Agent 统一配置

本工具管理 **Cursor / Codex / Claude Code / Hermes** 共享的 skills、rules、MCP。资产在 **`~/.ai-config/`**，不在本仓库。

## 快速安装

```bash
cd ~/Code/zh-cloud/ai-config
cargo build -p ai-config-cli --release
./target/release/ai-config install
```

## 资产路径

| 类型   | 路径                                  |
| ------ | ------------------------------------- |
| Skills | `~/.ai-config/skills/<name>/SKILL.md` |
| Rules  | `~/.ai-config/rules/*.mdc`            |
| MCP    | `~/.ai-config/mcp/servers/*.json`     |
| 密钥   | `~/.config/ai-config/secrets.env`     |

## 文档

- [README.md](./README.md)
- [docs/README.md](./docs/README.md)
