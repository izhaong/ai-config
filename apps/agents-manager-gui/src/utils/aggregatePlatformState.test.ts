import { describe, expect, it } from "vitest";

import type { PlatformAssetEntry } from "../types";
import { aggregatePlatformState } from "./aggregatePlatformState";

function entry(
  states: Partial<PlatformAssetEntry["states"]>,
): PlatformAssetEntry {
  return {
    name: "demo",
    kind: "skill",
    description: "",
    platform_path: "",
    states: {
      agentsmanager: "unlinked",
      cursor: "unlinked",
      codex: "unlinked",
      claude: "unlinked",
      hermes: "unlinked",
      ...states,
    },
  };
}

describe("aggregatePlatformState", () => {
  it("returns none when no entries", () => {
    expect(aggregatePlatformState([], "cursor")).toBe("none");
  });

  it("returns all when every selected row is linked", () => {
    const rows = [entry({ cursor: "linked" }), entry({ cursor: "linked" })];
    expect(aggregatePlatformState(rows, "cursor")).toBe("all");
  });

  it("returns none when no row is linked", () => {
    const rows = [entry({ cursor: "unlinked" }), entry({ cursor: "missing" })];
    expect(aggregatePlatformState(rows, "cursor")).toBe("none");
  });

  it("returns partial for mixed selection", () => {
    const rows = [entry({ cursor: "linked" }), entry({ cursor: "unlinked" })];
    expect(aggregatePlatformState(rows, "cursor")).toBe("partial");
  });

  it("counts synced as active for batch aggregation", () => {
    const rows = [
      { ...entry({ cursor: "synced" }), kind: "mcp" as const },
      { ...entry({ cursor: "synced" }), kind: "mcp" as const },
    ];
    expect(aggregatePlatformState(rows, "cursor")).toBe("all");
  });
});
