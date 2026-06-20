import { describe, expect, it } from "vitest";

import type { PlatformAssetEntry } from "../types";
import {
  resolveBatchEntryPlatformAction,
  resolveBatchPlatformToggleMode,
  resolveEntryPlatformToggleAction,
  type EntryPlatformToggleContext,
} from "./entryPlatformToggle";

function entry(
  states: Partial<PlatformAssetEntry["states"]>,
): PlatformAssetEntry {
  return {
    name: "demo",
    kind: "skill",
    description: "",
    platform_path: "/tmp/demo",
    states: {
      aiconfig: "unlinked",
      cursor: "unlinked",
      codex: "unlinked",
      claude: "unlinked",
      hermes: "unlinked",
      ...states,
    },
  };
}

const sourceView: EntryPlatformToggleContext = {
  activePlatform: "aiconfig",
  browsingSource: true,
  issueKey: () => false,
};

const cursorView: EntryPlatformToggleContext = {
  activePlatform: "cursor",
  browsingSource: false,
  issueKey: () => false,
};

describe("resolveEntryPlatformToggleAction", () => {
  it("已链接时收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ cursor: "linked", aiconfig: "linked" }),
        "cursor",
        sourceView,
      ),
    ).toBe("retract");
  });

  it("未链接时在源视图下发", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ cursor: "unlinked", aiconfig: "linked" }),
        "cursor",
        sourceView,
      ),
    ).toBe("deploy");
  });

  it("aiconfig 已纳管时 icon 不删源", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ aiconfig: "linked" }),
        "aiconfig",
        cursorView,
      ),
    ).toBe("skip");
  });
});

describe("resolveBatchPlatformToggleMode", () => {
  it("全部已激活 → retract_all", () => {
    const rows = [
      entry({ cursor: "linked", aiconfig: "linked" }),
      entry({ cursor: "linked", aiconfig: "linked" }),
    ];
    expect(resolveBatchPlatformToggleMode(rows, "cursor")).toBe("retract_all");
  });

  it("部分或全未激活 → activate_all", () => {
    const partial = [
      entry({ cursor: "linked", aiconfig: "linked" }),
      entry({ cursor: "unlinked", aiconfig: "linked" }),
    ];
    const none = [
      entry({ cursor: "unlinked", aiconfig: "linked" }),
      entry({ cursor: "missing", aiconfig: "linked" }),
    ];
    expect(resolveBatchPlatformToggleMode(partial, "cursor")).toBe(
      "activate_all",
    );
    expect(resolveBatchPlatformToggleMode(none, "cursor")).toBe("activate_all");
  });
});

describe("resolveBatchEntryPlatformAction", () => {
  it("activate_all：已激活项跳过，未激活项下发", () => {
    const linked = entry({ cursor: "linked", aiconfig: "linked" });
    const unlinked = entry({ cursor: "unlinked", aiconfig: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "cursor",
        "activate_all",
        sourceView,
      ),
    ).toBe("skip");
    expect(
      resolveBatchEntryPlatformAction(
        unlinked,
        "cursor",
        "activate_all",
        sourceView,
      ),
    ).toBe("deploy");
  });

  it("retract_all：仅已激活项收回（IDE 平台）", () => {
    const linked = entry({ cursor: "linked", aiconfig: "linked" });
    const unlinked = entry({ cursor: "unlinked", aiconfig: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "cursor",
        "retract_all",
        cursorView,
      ),
    ).toBe("retract");
    expect(
      resolveBatchEntryPlatformAction(
        unlinked,
        "cursor",
        "retract_all",
        cursorView,
      ),
    ).toBe("skip");
  });

  it("retract_all：ai-config 批量永不删源", () => {
    const linked = entry({ aiconfig: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "aiconfig",
        "retract_all",
        cursorView,
      ),
    ).toBe("skip");
  });
});
