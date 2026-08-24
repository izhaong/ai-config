export type AssetKind = "skill" | "rule" | "mcp" | "agent" | "command" | "hook";
/** 5 平台：agents-manager 为资产源，其余为 IDE 下发目标 */
export type Platform = "agentsmanager" | "cursor" | "codex" | "claude" | "hermes";
/** 可向 IDE deploy / retract 的 4 个目标 */
export type DeployPlatform = Exclude<Platform, "agentsmanager">;
export type LinkState = "linked" | "synced" | "unlinked" | "broken" | "missing";

/** Source-first planner ownership state. `synced` is deliberately absent: equal content alone
 * never grants ownership or permission to retract. */
export type ProjectionState =
  | "missing"
  | "managed_link"
  | "managed_generated"
  | "copied"
  | "equivalent"
  | "foreign"
  | "conflict"
  | "drifted"
  | "unsupported";

export interface ProjectionAction {
  action_id: string;
  kind: string;
  state?: ProjectionState | "skipped";
  reason_code: string;
  target?: { path?: string } | null;
  members: Array<{
    source: { layer: string; absolute_path: string };
  }>;
  mcp_members: Array<{
    name: string;
    secret_keys: string[];
    missing_secret_keys: string[];
  }>;
}

export interface ProjectionReview {
  schema_version: number;
  plan_digest: string;
  actions: ProjectionAction[];
  blocking_reason: string | null;
  ledger_status?: "ledger_unavailable";
}

export interface ProjectionApply {
  changed: number;
  unchanged: number;
  skipped: number;
  conflict: number;
  failed: number;
  rolled_back: number;
  rollback_failed: number;
  not_applied: number;
}

export interface ImportPlan {
  schema_version: number;
  plan_digest: string;
  actions: Array<{
    action_id: string;
    kind: AssetKind;
    name: string;
    source_platform: DeployPlatform;
    source_path: string;
    destination_layer: string;
    destination_path: string;
    normalized_diff: { change: string };
    secret_preflight: { status: "clear" | "blocked"; key_names: string[] };
    blocking_reasons: string[];
  }>;
}

export interface ImportApply {
  transaction_id?: string | null;
  applied: number;
  skipped: number;
}

export type HookLifecycleGroup = "agent" | "tab" | "workspace";

export interface HookLifecycleView {
  lifecycle: string;
  group: HookLifecycleGroup;
  label: string;
  short_label: string;
  active: boolean;
  supported: boolean;
  /** 支持该生命周期的 IDE 平台（全平台并集） */
  supported_platforms?: Platform[];
}

export interface PlatformAssetEntry {
  name: string;
  kind: AssetKind;
  description: string;
  platform_path: string;
  states: Record<Platform, LinkState>;
  hook_lifecycles?: HookLifecycleView[];
  /** Hook 条目类型：`command` | `prompt` */
  hook_type?: "command" | "prompt";
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
  /** Read-only hints for old asset roots (~/.ai-config, ~/.agents-manager). */
  legacy_asset_roots?: string[];
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
  "agentsmanager",
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
  "hook",
];

export function isSourcePlatform(platform: Platform): boolean {
  return platform === "agentsmanager";
}

export function canDeploy(state: LinkState): boolean {
  return (
    state === "unlinked" ||
    state === "broken" ||
    state === "missing"
  );
}

export function canRetract(state: LinkState): boolean {
  return state === "linked";
}

/** 平台视图删除：只有存在 ownership 证据的下发才可收回。 */
export function canRemoveFromPlatform(
  kind: AssetKind,
  state: LinkState,
): boolean {
  return kind !== "mcp" && state === "linked";
}

export function isPlatformActive(state: LinkState): boolean {
  return state === "linked" || state === "synced";
}

export function isManagedPlatformState(state: LinkState): boolean {
  return state === "linked";
}

export function hasSourceEntry(entry: PlatformAssetEntry): boolean {
  return isPlatformActive(entry.states?.agentsmanager ?? "unlinked");
}
