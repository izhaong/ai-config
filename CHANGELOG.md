# Changelog

All notable changes to this project will be documented in this file.

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.1.0] - 2026-06-14

### Added

- GitHub CI: fmt / clippy / test on Ubuntu & macOS; GUI frontend smoke build.
- GitHub Release workflow: tag `vX.Y.Z` builds CLI (Linux/macOS/Windows) + Tauri GUI installers; release notes from `CHANGELOG.md`.
- GitHub open-source docs: README, CONTRIBUTING, issue/PR templates.
- MIT `LICENSE`.

### Changed

- MCP user assets consolidated to a **single** `~/.ai-config/mcp.json` (legacy `mcp/servers/` migrated on install).
- Test MCP fixtures sanitized (placeholders only; no real tokens or private hosts).

### Security

- Documented vulnerability reporting via GitHub Security Advisories.

[0.1.0]: https://github.com/izhaong/ai-config/releases/tag/v0.1.0
