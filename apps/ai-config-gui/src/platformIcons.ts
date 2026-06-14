import type { Platform } from "./types";

/** 各平台官方 favicon（`public/platforms/`，Vite 静态资源） */
export const PLATFORM_FAVICON: Record<Platform, string> = {
  cursor: "/platforms/cursor.png",
  codex: "/platforms/codex.png",
  claude: "/platforms/claude.png",
  hermes: "/platforms/hermes.png",
};

export const PLATFORM_NAME: Record<Platform, string> = {
  cursor: "Cursor",
  codex: "Codex",
  claude: "Claude Code",
  hermes: "Hermes",
};
