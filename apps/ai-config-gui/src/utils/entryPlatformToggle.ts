import type {
  AssetKind,
  DeployPlatform,
  Platform,
  PlatformAssetEntry,
} from "../types";
import {
  canDeploy,
  canRetract,
  hasSourceEntry,
  isSourcePlatform,
} from "../types";
import {
  aggregatePlatformState,
} from "./aggregatePlatformState";

export type EntryPlatformToggleAction =
  | "deploy"
  | "retract"
  | "import"
  | "delete_source"
  | "skip"
  | "unsupported";

export type BatchPlatformToggleMode = "activate_all" | "retract_all";

export interface EntryPlatformToggleContext {
  activePlatform: Platform;
  browsingSource: boolean;
  /** `${deployPlatform}:${assetKind}` */
  issueKey: (plat: DeployPlatform, kind: AssetKind) => boolean;
}

/** 批量：仅全部已激活 → 收回；部分/全未激活 → 全部下发 */
export function resolveBatchPlatformToggleMode(
  entries: PlatformAssetEntry[],
  plat: Platform,
): BatchPlatformToggleMode {
  return aggregatePlatformState(entries, plat) === "all"
    ? "retract_all"
    : "activate_all";
}

/** 与单行平台 icon 点击一致：已激活 → 收回，未激活 → 下发/导入 */
export function resolveEntryPlatformToggleAction(
  entry: PlatformAssetEntry,
  plat: Platform,
  ctx: EntryPlatformToggleContext,
): EntryPlatformToggleAction {
  if (plat === "aiconfig") {
    const state = entry.states.aiconfig;
    // 源平台 icon 仅支持「导入到源」；删源只能走删除按钮（源视图 + 确认）
    if (canRetract(state)) {
      return "skip";
    }
    if (canDeploy(state) && !isSourcePlatform(ctx.activePlatform)) {
      return "import";
    }
    return "skip";
  }

  const deployPlat = plat as DeployPlatform;
  if (ctx.issueKey(deployPlat, entry.kind)) {
    return "unsupported";
  }

  const state = entry.states[deployPlat];
  if (canRetract(state)) {
    return "retract";
  }
  if (!canDeploy(state)) {
    return "skip";
  }

  if (ctx.browsingSource) {
    return "deploy";
  }

  if (isSourcePlatform(ctx.activePlatform)) {
    return "skip";
  }

  if (ctx.activePlatform === deployPlat) {
    return "deploy";
  }

  if (hasSourceEntry(entry)) {
    return "deploy";
  }

  return "skip";
}

/** 批量模式：按聚合态决定每项只参与下发或只参与收回 */
export function resolveBatchEntryPlatformAction(
  entry: PlatformAssetEntry,
  plat: Platform,
  mode: BatchPlatformToggleMode,
  ctx: EntryPlatformToggleContext,
): EntryPlatformToggleAction {
  // ai-config 源无「收回」：批量 icon 只允许导入，删源须走删除按钮 + 确认框
  if (plat === "aiconfig" && mode === "retract_all") {
    return "skip";
  }

  const single = resolveEntryPlatformToggleAction(entry, plat, ctx);
  if (single === "skip" || single === "unsupported" || single === "delete_source") {
    return "skip";
  }
  if (mode === "retract_all") {
    return single === "retract" ? single : "skip";
  }
  return single === "deploy" || single === "import" ? single : "skip";
}
