# Project MCP source

Keep each project-owned MCP server as `mcp/servers/<name>.json`. Source files may contain only
portable fields and secret references; real secret values belong in the local `secrets.env` store
and must never be committed. This repository intentionally carries no live MCP server fixture.
