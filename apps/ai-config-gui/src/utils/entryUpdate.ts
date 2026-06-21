import type { AssetKind, DeployPlatform, PlatformAssetEntry } from "../types";
import { DEPLOY_PLATFORMS, hasSourceEntry, isPlatformActive } from "../types";

/** 已有平台副本、可从 ai-config 源重新下发的目标平台 */
export function deployPlatformsForUpdate(
  entry: PlatformAssetEntry,
): DeployPlatform[] {
  if (!hasSourceEntry(entry)) {
    return [];
  }
  return DEPLOY_PLATFORMS.filter((plat) =>
    isPlatformActive(entry.states[plat]),
  );
}

export function canUpdateEntry(
  entry: PlatformAssetEntry,
  issueKey?: (plat: DeployPlatform, kind: AssetKind) => boolean,
): boolean {
  return deployPlatformsForUpdate(entry).some(
    (plat) => !issueKey?.(plat, entry.kind),
  );
}
