import { describe, expect, it } from "vitest";

import type { PlatformAssetEntry } from "../types";
import { canUpdateEntry, deployPlatformsForUpdate } from "./entryUpdate";

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

describe("entryUpdate", () => {
  it("无源时不更新", () => {
    expect(deployPlatformsForUpdate(entry({ cursor: "linked" }))).toEqual([]);
    expect(canUpdateEntry(entry({ cursor: "linked" }))).toBe(false);
  });

  it("有源且平台已激活时列入更新目标", () => {
    const e = entry({
      agentsmanager: "linked",
      cursor: "linked",
      codex: "synced",
      claude: "unlinked",
    });
    expect(deployPlatformsForUpdate(e)).toEqual(["cursor", "codex"]);
    expect(canUpdateEntry(e)).toBe(true);
  });

  it("平台能力受限时不可更新", () => {
    const e = entry({ agentsmanager: "linked", cursor: "linked" });
    expect(canUpdateEntry(e, (plat) => plat === "cursor")).toBe(false);
  });
});
