export type AssetKind = "skill" | "rule" | "mcp" | "agent";
export type Platform = "cursor" | "codex" | "claude" | "hermes";
export type LinkState = "linked" | "unlinked" | "broken" | "missing";

export interface AssetEntry {
  name: string;
  kind: AssetKind;
  description: string;
  source_path: string;
  states: Record<Platform, LinkState>;
}

export interface AssetList {
  entries: AssetEntry[];
  broken_links: number;
}

export interface ProjectItem {
  id: number;
  name: string;
  root_path: string;
  registered_at: string;
}

export interface DoctorSummary {
  broken: number;
  wrong_source: number;
  wrong_type: number;
  missing_secrets: string[];
  unregistered_projects: string[];
  platform_capability_issues: Array<{
    platform: Platform;
    kind: AssetKind;
    reason: string;
  }>;
  exit_code: number;
}

export interface AssetDetail {
  name: string;
  description: string;
  content: string;
  source_path: string;
  parent_path: string;
}

/** @deprecated 使用 AssetDetail */
export type SkillDetail = AssetDetail;

export const PLATFORMS: Platform[] = ["cursor", "codex", "claude", "hermes"];

export const ASSET_KINDS: AssetKind[] = ["skill", "rule", "mcp", "agent"];

export const ASSET_LABEL: Record<AssetKind, string> = {
  skill: "Skills",
  rule: "Rules",
  mcp: "MCP",
  agent: "Agents",
};

export function canDeploy(state: LinkState): boolean {
  return state === "unlinked" || state === "broken" || state === "missing";
}

export function canRetract(state: LinkState): boolean {
  return state === "linked";
}

/** UI 仅两种态：已同步到平台 = 激活，否则 = 未激活 */
export function isPlatformActive(state: LinkState): boolean {
  return state === "linked";
}

export function platformUiLabel(active: boolean): string {
  return active ? "已激活" : "未激活";
}

export function linkStateLabel(state: LinkState): string {
  switch (state) {
    case "linked":
      return "已同步";
    case "unlinked":
      return "未同步";
    case "broken":
      return "链接异常";
    case "missing":
      return "目标缺失";
  }
}
