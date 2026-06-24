export type AssetKind = "skill" | "rule" | "mcp" | "agent" | "command";
/** 5 平台：ai-config 为资产源，其余为 IDE 下发目标 */
export type Platform = "aiconfig" | "cursor" | "codex" | "claude" | "hermes";
/** 可向 IDE deploy / retract 的 4 个目标 */
export type DeployPlatform = Exclude<Platform, "aiconfig">;
export type LinkState = "linked" | "synced" | "unlinked" | "broken" | "missing";

export interface PlatformAssetEntry {
  name: string;
  kind: AssetKind;
  description: string;
  platform_path: string;
  states: Record<Platform, LinkState>;
}

export interface PlatformAssetList {
  entries: PlatformAssetEntry[];
  platform: Platform;
}

export interface PlatformKindPath {
  platform: Platform;
  path: string;
  supported: boolean;
}

/** 某平台上同名资产的可读内容（冲突对比用） */
export interface PlatformAssetVariant {
  platform: Platform;
  platform_label: string;
  content: string;
  content_summary: string;
  differs_from_baseline: boolean;
}

/** 跨平台同步冲突：目标已有同名但不同内容的副本 */
export interface SyncConflictReport {
  kind: AssetKind;
  name: string;
  baseline_platform: Platform;
  target_platform: Platform;
  variants: PlatformAssetVariant[];
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
    platform: DeployPlatform;
    kind: AssetKind;
    reason: string;
  }>;
  exit_code: number;
}

export interface GitEnsureOutcome {
  git_available: boolean;
  was_repo: boolean;
  just_initialized: boolean;
  initial_commit: boolean;
}

export interface GitRepoStatus {
  git_available: boolean;
  is_repo: boolean;
  just_initialized: boolean;
  branch?: string | null;
  dirty: boolean;
  dirty_count: number;
  has_remote: boolean;
  remote_url?: string | null;
  ahead: number;
  behind: number;
}

export interface GitSyncConfig {
  remote_url?: string | null;
  branch: string;
}

export interface GitSyncOutcome {
  committed: boolean;
  pulled: boolean;
  pushed: boolean;
  message: string;
}

export interface GitBootstrapResponse {
  asset_root: string;
  outcome: GitEnsureOutcome;
  status: GitRepoStatus;
  config: GitSyncConfig;
}

export interface AssetDetail {
  name: string;
  description: string;
  content: string;
  source_path: string;
  parent_path: string;
}

/** GUI 平台 dock 固定 5 个 */
export const ALL_PLATFORMS: Platform[] = [
  "aiconfig",
  "cursor",
  "codex",
  "claude",
  "hermes",
];

export const DEPLOY_PLATFORMS: DeployPlatform[] = [
  "cursor",
  "codex",
  "claude",
  "hermes",
];

export const ASSET_KINDS: AssetKind[] = [
  "skill",
  "rule",
  "mcp",
  "agent",
  "command",
];

export function isSourcePlatform(platform: Platform): boolean {
  return platform === "aiconfig";
}

export function canDeploy(state: LinkState): boolean {
  return (
    state === "unlinked" ||
    state === "broken" ||
    state === "missing" ||
    state === "synced"
  );
}

export function canRetract(state: LinkState): boolean {
  return state === "linked";
}

/** 平台视图删除：可移除该平台上的副本（含 synced / 外部安装） */
export function canRemoveFromPlatform(state: LinkState): boolean {
  return (
    state === "linked" ||
    state === "synced" ||
    state === "unlinked" ||
    state === "broken"
  );
}

export function isPlatformActive(state: LinkState): boolean {
  return state === "linked" || state === "synced";
}

export function isManagedPlatformState(state: LinkState): boolean {
  return state === "linked";
}

export function hasSourceEntry(entry: PlatformAssetEntry): boolean {
  return isPlatformActive(entry.states?.aiconfig ?? "unlinked");
}
