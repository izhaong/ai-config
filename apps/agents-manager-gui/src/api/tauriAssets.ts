import { invoke } from "@tauri-apps/api/core";

import type {
  AssetDetail,
  AssetKind,
  DeployPlatform,
  DoctorSummary,
  Platform,
  PlatformAssetList,
  PlatformKindPath,
  ProjectionApply,
  ProjectionReview,
  ImportApply,
  ImportPlan,
  ProjectItem,
} from "../types";

export function fetchDoctor(): Promise<DoctorSummary> {
  return invoke<DoctorSummary>("cmd_doctor");
}

export function fetchProjectionPlan(
  project: string,
  retract = false,
): Promise<ProjectionReview> {
  return invoke<ProjectionReview>("cmd_projection_plan", { project, retract });
}

export function applyProjectionPlan(
  project: string,
  planDigest: string,
): Promise<ProjectionApply> {
  return invoke<ProjectionApply>("cmd_projection_apply", {
    project,
    planDigest,
  });
}

export function retractProjectionPlan(
  project: string,
  planDigest: string,
): Promise<ProjectionApply> {
  return invoke<ProjectionApply>("cmd_projection_retract", {
    project,
    planDigest,
  });
}

export function fetchImportToSourcePlan(
  kind: AssetKind,
  name: string,
  project: string,
  fromPlatform: DeployPlatform,
  replace = false,
): Promise<ImportPlan> {
  return invoke<ImportPlan>("cmd_import_to_source_plan", {
    kind,
    name,
    project,
    fromPlatform,
    replace,
  });
}

export function applyImportToSourcePlan(
  kind: AssetKind,
  name: string,
  project: string,
  fromPlatform: DeployPlatform,
  planDigest: string,
  selectedActionIds: string[],
  replace = false,
): Promise<ImportApply> {
  return invoke<ImportApply>("cmd_import_to_source_apply", {
    kind,
    name,
    project,
    fromPlatform,
    replace,
    planDigest,
    selectedActionIds,
  });
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
