# Security Policy

## Supported versions

| Version | Supported |
| --- | --- |
| `0.1.x` | Yes |

## Reporting a vulnerability

Please **do not** open a public issue for security-sensitive reports.

Preferred:

1. [GitHub Private Security Advisories](https://github.com/izhaong/agents-manager/security/advisories/new) for this repository, **or**
2. Email **security@example.com** (maintainer — replace with your address) with:
   - Description and impact
   - Steps to reproduce
   - Affected version/commit

We aim to acknowledge reports within **72 hours** and will coordinate disclosure after a fix is available.

## Secrets handling

- Store credentials only in `~/.config/agents-manager/secrets.env` (local, mode `0600`).
- Never commit `.env`, `secrets.env`, or real tokens in MCP JSON fixtures.
