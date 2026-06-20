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
} from "../types";

export function fetchDoctor(): Promise<DoctorSummary> {
  return invoke<DoctorSummary>("cmd_doctor");
}

export function fetchProjectsList(): Promise<ProjectItem[]> {
  return invoke<ProjectItem[]>("cmd_projects_list");
}

export function fetchPlatformKindPaths(
  project: string,
  kind: AssetKind
): Promise<PlatformKindPath[]> {
  return invoke<Array<{ platform: string; path: string; supported: boolean }>>(
    "cmd_platform_kind_paths",
    { project, kind }
  ).then((rows) =>
    rows.map((r) => ({
      platform: r.platform as Platform,
      path: r.path,
      supported: r.supported,
    }))
  );
}

export function fetchPlatformList(
  project: string,
  platform: Platform,
  kind: AssetKind
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
  project: string
): Promise<AssetDetail> {
  return invoke<AssetDetail>(`cmd_${kind}_get`, { name, project });
}

export function readPlatformAsset(
  path: string,
  kind: AssetKind,
  name: string
): Promise<AssetDetail> {
  return invoke<AssetDetail>("cmd_read_platform_asset", { path, kind, name });
}

export function saveAsset(
  kind: AssetKind,
  name: string,
  content: string,
  project: string
): Promise<string> {
  return invoke<string>(`cmd_${kind}_save`, { name, content, project });
}

export function deleteAsset(
  kind: AssetKind,
  name: string,
  project: string
): Promise<string> {
  return invoke<string>(`cmd_${kind}_delete`, { name, project });
}

export function deployAsset(
  kind: AssetKind,
  name: string,
  project: string,
  to: DeployPlatform
): Promise<string> {
  return invoke<string>(`cmd_${kind}_deploy`, { name, project, to });
}

export function retractAsset(
  kind: AssetKind,
  name: string,
  project: string,
  from: DeployPlatform
): Promise<string> {
  return invoke<string>(`cmd_${kind}_retract`, { name, project, from });
}

export function importAsset(
  kind: AssetKind,
  name: string,
  project: string,
  fromPlatform: Platform
): Promise<string> {
  return invoke<string>(`cmd_${kind}_import`, {
    name,
    project,
    fromPlatform,
  });
}

export function revealPath(path: string): Promise<void> {
  return invoke("cmd_reveal_path", { path });
}

export function addProject(name: string, rootPath: string): Promise<ProjectItem> {
  return invoke<ProjectItem>("cmd_projects_add", { name, rootPath });
}

export function removeProject(name: string): Promise<void> {
  return invoke("cmd_projects_remove", { name });
}
