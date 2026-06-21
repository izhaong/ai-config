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

const claudeView: EntryPlatformToggleContext = {
  activePlatform: "claude",
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

  it("aiconfig 已纳管时在其它平台视图收回 ai-config 副本", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ aiconfig: "linked" }),
        "aiconfig",
        cursorView,
      ),
    ).toBe("retract");
  });

  it("源视图浏览 ai-config 时点击 ai-config icon 不收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ aiconfig: "linked" }),
        "aiconfig",
        sourceView,
      ),
    ).toBe("skip");
  });

  it("平台视图点击当前浏览平台不收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ claude: "linked", cursor: "unlinked" }),
        "claude",
        claudeView,
      ),
    ).toBe("skip");
  });

  it("synced 态点击下发而非收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ hermes: "synced", claude: "linked" }),
        "hermes",
        claudeView,
      ),
    ).toBe("platform_copy");
  });
});

describe("resolveBatchPlatformToggleMode", () => {
  it("全部已激活 → retract_all", () => {
    const rows = [
      entry({ cursor: "linked", aiconfig: "linked" }),
      entry({ cursor: "linked", aiconfig: "linked" }),
    ];
    expect(
      resolveBatchPlatformToggleMode(rows, "cursor", "aiconfig", true),
    ).toBe("retract_all");
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
    expect(
      resolveBatchPlatformToggleMode(partial, "cursor", "aiconfig", true),
    ).toBe("activate_all");
    expect(
      resolveBatchPlatformToggleMode(none, "cursor", "aiconfig", true),
    ).toBe("activate_all");
  });

  it("平台视图浏览 Claude 时批量不收回 Claude", () => {
    const rows = [
      entry({ claude: "linked", cursor: "unlinked" }),
      entry({ claude: "linked", cursor: "linked" }),
    ];
    expect(
      resolveBatchPlatformToggleMode(rows, "claude", "claude", false),
    ).toBe("activate_all");
  });

  it("源视图浏览 ai-config 时批量不收回 ai-config", () => {
    const rows = [
      entry({ aiconfig: "linked" }),
      entry({ aiconfig: "linked" }),
    ];
    expect(
      resolveBatchPlatformToggleMode(rows, "aiconfig", "aiconfig", true),
    ).toBe("activate_all");
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
        sourceView,
      ),
    ).toBe("retract");
    expect(
      resolveBatchEntryPlatformAction(
        unlinked,
        "cursor",
        "retract_all",
        sourceView,
      ),
    ).toBe("skip");
  });

  it("retract_all：平台视图浏览 Cursor 时不收回 Cursor", () => {
    const linked = entry({ cursor: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "cursor",
        "retract_all",
        cursorView,
      ),
    ).toBe("skip");
  });

  it("retract_all：ai-config 批量收回平台副本", () => {
    const linked = entry({ aiconfig: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "aiconfig",
        "retract_all",
        cursorView,
      ),
    ).toBe("retract");
  });

  it("retract_all：源视图浏览 ai-config 时不批量收回", () => {
    const linked = entry({ aiconfig: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "aiconfig",
        "retract_all",
        sourceView,
      ),
    ).toBe("skip");
  });
});
