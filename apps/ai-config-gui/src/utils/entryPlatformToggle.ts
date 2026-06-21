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
import { aggregateManagedPlatformState } from "./aggregatePlatformState";

export type EntryPlatformToggleAction =
  | "deploy"
  | "retract"
  | "import"
  | "platform_copy"
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
  return aggregateManagedPlatformState(entries, plat) === "all"
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

    // 源视图浏览 ai-config：与其它平台一致，点击当前平台 icon 不收回
    if (ctx.browsingSource && ctx.activePlatform === "aiconfig" && canRetract(state)) {
      return "skip";
    }

    if (canRetract(state)) {
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

  // 平台视图下点击当前浏览平台：仅展示存在，不收回来源安装（如 npx skills 装进 Claude）
  if (isBrowsingDeployPlatform(ctx) && ctx.activePlatform === plat) {
    return "skip";
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

  if (isBrowsingDeployPlatform(ctx)) {
    return "platform_copy";
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
  return single === "deploy" ||
    single === "import" ||
    single === "platform_copy"
    ? single
    : "skip";
}
