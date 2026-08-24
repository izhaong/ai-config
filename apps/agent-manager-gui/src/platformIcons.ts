import type { Platform } from "./types";

/** 各平台图标（`public/`，Vite 静态资源） */
export const PLATFORM_FAVICON: Record<Platform, string> = {
  aiconfig: "/agent-manager.png",
  cursor: "/platforms/cursor.png",
  codex: "/platforms/codex.png",
  claude: "/platforms/claude.png",
  hermes: "/platforms/hermes.png",
};

export const PLATFORM_NAME: Record<Platform, string> = {
  aiconfig: "agent-manager",
  cursor: "Cursor",
  codex: "Codex",
  claude: "Claude Code",
  hermes: "Hermes",
};
