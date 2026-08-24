import type { AssetKind } from "../types";

export type RowSyncLeadSlot = "none" | "skill-menu" | "mcp-add";

export function rowSyncLeadSlot(
  kind: AssetKind,
  browsingSource: boolean,
): RowSyncLeadSlot {
  if (kind === "skill") return "skill-menu";
  if (kind === "mcp" && browsingSource) return "mcp-add";
  return "none";
}
