import type { LinkState, Platform, PlatformAssetEntry } from "../types";
import { isPlatformActive } from "../types";

/** 已选行在某平台上的聚合链接态（类似全选 checkbox 的 checked / indeterminate） */
export type AggregatePlatformState = "none" | "all" | "partial";

export function aggregatePlatformState(
  entries: PlatformAssetEntry[],
  plat: Platform,
): AggregatePlatformState {
  if (entries.length === 0) {
    return "none";
  }
  let linked = 0;
  for (const entry of entries) {
    if (isPlatformActive(entry.states[plat])) {
      linked += 1;
    }
  }
  if (linked === 0) {
    return "none";
  }
  if (linked === entries.length) {
    return "all";
  }
  return "partial";
}

export function aggregateLinkStateForUi(
  entries: PlatformAssetEntry[],
  plat: Platform,
): LinkState | "mixed" {
  const agg = aggregatePlatformState(entries, plat);
  if (agg === "all") {
    return "linked";
  }
  if (agg === "partial") {
    return "mixed";
  }
  return "unlinked";
}
