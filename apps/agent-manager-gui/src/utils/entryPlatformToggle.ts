import type {
  AssetKind,
  DeployPlatform,
  LinkState,
  Platform,
  PlatformAssetEntry,
} from "../types";
import {
  canDeploy,
  canRetract,
  hasSourceEntry,
  isPlatformActive,
  isSourcePlatform,
} from "../types";
import { aggregateManagedPlatformState } from "./aggregatePlatformState";

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

function isBrowsingDeployPlatform(ctx: EntryPlatformToggleContext): boolean {
  return !ctx.browsingSource && !isSourcePlatform(ctx.activePlatform);
}

/** MCP 没有 per-server ownership 证据，不能触发收回。 */
function canRetractEntry(entry: PlatformAssetEntry, state: LinkState): boolean {
  if (entry.kind === "mcp") {
    return false;
  }
  if (entry.kind === "hook") {
    return isPlatformActive(state);
  }
  return canRetract(state);
}

/** 平台视图下禁止点击「当前浏览平台」icon（仅展示状态） */
function skipBrowseCurrentToggle(
  ctx: EntryPlatformToggleContext,
  plat: Platform,
): boolean {
  if (
    ctx.browsingSource &&
    ctx.activePlatform === "aiconfig" &&
    plat === "aiconfig"
  ) {
    return true;
  }
  return isBrowsingDeployPlatform(ctx) && ctx.activePlatform === plat;
}

/** 批量：仅全部已激活 → 收回；部分/全未激活 → 全部下发 */
export function resolveBatchPlatformToggleMode(
  entries: PlatformAssetEntry[],
  plat: Platform,
  activePlatform: Platform,
  browsingSource: boolean,
): BatchPlatformToggleMode {
  if (
    isBrowsingDeployPlatform({
      activePlatform,
      browsingSource,
      issueKey: () => false,
    }) &&
    plat === activePlatform
  ) {
    return "activate_all";
  }
  if (browsingSource && activePlatform === "aiconfig" && plat === "aiconfig") {
    return "activate_all";
  }
  const agg = aggregateManagedPlatformState(entries, plat);
  return agg === "all" ? "retract_all" : "activate_all";
}

/** 与单行平台 icon 点击一致：已激活 → 收回，未激活 → 下发/导入 */
export function resolveEntryPlatformToggleAction(
  entry: PlatformAssetEntry,
  plat: Platform,
  ctx: EntryPlatformToggleContext,
): EntryPlatformToggleAction {
  if (plat === "aiconfig") {
    const state = entry.states.aiconfig;

    if (
      ctx.browsingSource &&
      ctx.activePlatform === "aiconfig" &&
      canRetractEntry(entry, state)
    ) {
      return "skip";
    }

    if (canRetractEntry(entry, state)) {
      return "retract";
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

  // 当前浏览平台 icon 仅展示状态，不触发下发/收回
  if (skipBrowseCurrentToggle(ctx, plat)) {
    return "skip";
  }

  // Platform views are inventory/import surfaces only. Projection and retraction are always
  // scope-level, digest-bound operations opened from the canonical source view.
  if (!ctx.browsingSource) {
    return "skip";
  }

  const state = entry.states[deployPlat];
  if (canRetractEntry(entry, state)) {
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
  const single = resolveEntryPlatformToggleAction(entry, plat, ctx);
  if (
    single === "skip" ||
    single === "unsupported" ||
    single === "delete_source"
  ) {
    return "skip";
  }
  if (mode === "retract_all") {
    return single === "retract" ? single : "skip";
  }
  return single === "deploy" || single === "import" ? single : "skip";
}
