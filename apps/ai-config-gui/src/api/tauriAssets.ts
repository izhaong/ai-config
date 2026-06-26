import { invoke } from "@tauri-apps/api/core";

import type {
  AssetDetail,
  AssetKind,
  DeployPlatform,
  DoctorSummary,
  Platform,
  PlatformAssetList,
  PlatformKindPath,
  ProjectItem,
  SyncConflictReport,
} from "../types";

export function fetchDoctor(): Promise<DoctorSummary> {
  return invoke<DoctorSummary>("cmd_doctor");
}

export function fetchProjectsList(): Promise<ProjectItem[]> {
  return invoke<ProjectItem[]>("cmd_projects_list");
}

export function fetchPlatformKindPaths(
  project: string,
  kind: AssetKind,
): Promise<PlatformKindPath[]> {
  return invoke<Array<{ platform: string; path: string; supported: boolean }>>(
    "cmd_platform_kind_paths",
    { project, kind },
  ).then((rows) =>
    rows.map((r) => ({
      platform: r.platform as Platform,
      path: r.path,
      supported: r.supported,
    })),
  );
}

export function fetchPlatformList(
  project: string,
  platform: Platform,
  kind: AssetKind,
): Promise<PlatformAssetList> {
  return invoke<PlatformAssetList>("cmd_list_platform", {
    project,
    platform,
    kind,
  });
}

export function fetchAsset(
  kind: AssetKind,
  name: string,
  project: string,
): Promise<AssetDetail> {
  return invoke<AssetDetail>(`cmd_${kind}_get`, { name, project });
}

export function readPlatformAsset(
  path: string,
  kind: AssetKind,
  name: string,
): Promise<AssetDetail> {
  return invoke<AssetDetail>("cmd_read_platform_asset", { path, kind, name });
}

export function saveAsset(
  kind: AssetKind,
  name: string,
  content: string,
  project: string,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_save`, { name, content, project });
}

export function deleteAsset(
  kind: AssetKind,
  name: string,
  project: string,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_delete`, { name, project });
}

export function retractSourceAsset(
  kind: AssetKind,
  name: string,
  project: string,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_retract_source`, { name, project });
}

export function deployAssetFromPlatform(
  kind: AssetKind,
  name: string,
  project: string,
  from: DeployPlatform,
  to: DeployPlatform,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_deploy_from_platform`, {
    name,
    project,
    from,
    to,
  });
}

export function deployAsset(
  kind: AssetKind,
  name: string,
  project: string,
  to: DeployPlatform,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_deploy`, { name, project, to });
}

export function retractAsset(
  kind: AssetKind,
  name: string,
  project: string,
  from: Platform,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_retract`, { name, project, from });
}

export function importAsset(
  kind: AssetKind,
  name: string,
  project: string,
  fromPlatform: Platform,
): Promise<string> {
  return invoke<string>(`cmd_${kind}_import`, {
    name,
    project,
    fromPlatform,
  });
}

export function toggleHookLifecycle(
  name: string,
  lifecycle: string,
  enabled: boolean,
  project: string,
  platform: Platform,
): Promise<string> {
  return invoke<string>("cmd_hook_toggle_lifecycle", {
    name,
    lifecycle,
    enabled,
    project,
    platform,
  });
}

export function addSkillFromRemote(
  source: string,
  skillName: string | undefined,
  project: string,
  targetPlatform: Platform,
): Promise<string> {
  return invoke<string>("cmd_skill_add", {
    source,
    skillName: skillName ?? null,
    project,
    targetPlatform,
  });
}

export function revealPath(path: string): Promise<void> {
  return invoke("cmd_reveal_path", { path });
}

export function detectSyncConflict(
  kind: AssetKind,
  name: string,
  project: string,
  baselinePlatform: Platform,
  targetPlatform: Platform,
): Promise<SyncConflictReport | null> {
  return invoke<SyncConflictReport | null>("cmd_detect_sync_conflict", {
    kind,
    name,
    project,
    baselinePlatform,
    targetPlatform,
  });
}

export function applySyncChoice(
  kind: AssetKind,
  name: string,
  project: string,
  sourcePlatform: Platform,
  targetPlatform: Platform,
): Promise<string> {
  return invoke<string>("cmd_apply_sync_choice", {
    kind,
    name,
    project,
    sourcePlatform,
    targetPlatform,
  });
}

export async function pickProjectDirectory(
  title?: string,
): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    directory: true,
    multiple: false,
    title,
  });
  if (selected === null) return null;
  return typeof selected === "string" ? selected : (selected[0] ?? null);
}

export function addProject(
  name: string,
  rootPath: string,
): Promise<ProjectItem> {
  return invoke<ProjectItem>("cmd_projects_add", { name, rootPath });
}

export function removeProject(name: string): Promise<void> {
  return invoke("cmd_projects_remove", { name });
}

export interface AssetTransferItem {
  kind: AssetKind;
  name: string;
}

export function transferAssets(
  fromProject: string,
  toProject: string,
  items: AssetTransferItem[],
): Promise<string> {
  return invoke<string>("cmd_assets_transfer", {
    fromProject,
    toProject,
    items: items.map((item) => ({ kind: item.kind, name: item.name })),
  });
}
