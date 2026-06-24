import type { Platform, PlatformAssetEntry } from "../types";
import { hasSourceEntry, isSourcePlatform } from "../types";

/** 详情应从 ai-config 源读取（否则走平台路径只读预览） */
export function shouldLoadFromSource(
  entry: PlatformAssetEntry,
  activePlatform: Platform,
): boolean {
  if (isSourcePlatform(activePlatform)) {
    return entry.kind !== "mcp" || hasSourceEntry(entry);
  }
  return hasSourceEntry(entry);
}

/** 是否可在抽屉中打开资产 */
export function canOpenEntry(
  entry: PlatformAssetEntry,
  _activePlatform: Platform,
): boolean {
  const path = entry.platform_path?.trim() ?? "";
  if (!path) return false;
  return true;
}
