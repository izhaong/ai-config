import type { AssetKind } from "../types";

export function entryKey(entry: { kind: AssetKind; name: string }): string {
  return `${entry.kind}:${entry.name}`;
}
