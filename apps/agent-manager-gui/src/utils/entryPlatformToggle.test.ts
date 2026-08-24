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
      agentmanager: "unlinked",
      cursor: "unlinked",
      codex: "unlinked",
      claude: "unlinked",
      hermes: "unlinked",
      ...states,
    },
  };
}

const sourceView: EntryPlatformToggleContext = {
  activePlatform: "agentmanager",
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
        entry({ cursor: "linked", agentmanager: "linked" }),
        "cursor",
        sourceView,
      ),
    ).toBe("retract");
  });

  it("未链接时在源视图下发", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ cursor: "unlinked", agentmanager: "linked" }),
        "cursor",
        sourceView,
      ),
    ).toBe("deploy");
  });

  it("agentmanager 已纳管时在其它平台视图收回 agents-manager 副本", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ agentmanager: "linked" }),
        "agentmanager",
        cursorView,
      ),
    ).toBe("retract");
  });

  it("源视图浏览 agents-manager 时 skill 点击 agents-manager icon 不收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ agentmanager: "linked" }),
        "agentmanager",
        sourceView,
      ),
    ).toBe("skip");
  });

  it("源视图浏览 agents-manager 时 mcp 点击 agents-manager icon 仅展示状态", () => {
    expect(
      resolveEntryPlatformToggleAction(
        { ...entry({ agentmanager: "linked" }), kind: "mcp" },
        "agentmanager",
        sourceView,
      ),
    ).toBe("skip");
  });

  it("平台视图 mcp 点击当前浏览平台仅展示状态", () => {
    expect(
      resolveEntryPlatformToggleAction(
        { ...entry({ cursor: "synced" }), kind: "mcp" },
        "cursor",
        cursorView,
      ),
    ).toBe("skip");
  });

  it("平台视图 skill 点击当前浏览平台不收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ claude: "linked", cursor: "unlinked" }),
        "claude",
        claudeView,
      ),
    ).toBe("skip");
  });

  it("mcp synced 态不隐式收回或接管", () => {
    expect(
      resolveEntryPlatformToggleAction(
        { ...entry({ hermes: "synced", claude: "linked" }), kind: "mcp" },
        "hermes",
        claudeView,
      ),
    ).toBe("skip");
  });

  it("mcp linked 态在源视图不触发必失败的收回", () => {
    expect(
      resolveEntryPlatformToggleAction(
        {
          ...entry({ cursor: "linked", agentmanager: "linked" }),
          kind: "mcp",
        },
        "cursor",
        sourceView,
      ),
    ).toBe("skip");
  });

  it("mcp 无源时从 Cursor 视图不复制到 Codex", () => {
    expect(
      resolveEntryPlatformToggleAction(
        {
          ...entry({ cursor: "synced", codex: "missing" }),
          kind: "mcp",
        },
        "codex",
        cursorView,
      ),
    ).toBe("skip");
  });

  it("mcp 无源时从 Cursor 视图导入 agents-manager", () => {
    expect(
      resolveEntryPlatformToggleAction(
        {
          ...entry({ cursor: "synced", agentmanager: "unlinked" }),
          kind: "mcp",
        },
        "agentmanager",
        cursorView,
      ),
    ).toBe("import");
  });

  it("skill synced 态点击其它平台不隐式拷贝", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ hermes: "synced", claude: "linked" }),
        "hermes",
        claudeView,
      ),
    ).toBe("skip");
  });

  it("平台视图不执行投影；只有源视图能打开 source-first 计划", () => {
    expect(
      resolveEntryPlatformToggleAction(
        entry({ agentmanager: "linked", codex: "missing" }),
        "codex",
        cursorView,
      ),
    ).toBe("skip");
  });
});

describe("resolveBatchPlatformToggleMode", () => {
  it("全部已激活 → retract_all", () => {
    const rows = [
      entry({ cursor: "linked", agentmanager: "linked" }),
      entry({ cursor: "linked", agentmanager: "linked" }),
    ];
    expect(
      resolveBatchPlatformToggleMode(rows, "cursor", "agentmanager", true),
    ).toBe("retract_all");
  });

  it("部分或全未激活 → activate_all", () => {
    const partial = [
      entry({ cursor: "linked", agentmanager: "linked" }),
      entry({ cursor: "unlinked", agentmanager: "linked" }),
    ];
    const none = [
      entry({ cursor: "unlinked", agentmanager: "linked" }),
      entry({ cursor: "missing", agentmanager: "linked" }),
    ];
    expect(
      resolveBatchPlatformToggleMode(partial, "cursor", "agentmanager", true),
    ).toBe("activate_all");
    expect(
      resolveBatchPlatformToggleMode(none, "cursor", "agentmanager", true),
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

  it("源视图浏览 agents-manager 时批量不收回 agents-manager", () => {
    const rows = [
      entry({ agentmanager: "linked" }),
      entry({ agentmanager: "linked" }),
    ];
    expect(
      resolveBatchPlatformToggleMode(rows, "agentmanager", "agentmanager", true),
    ).toBe("activate_all");
  });

  it("全 mcp 且 synced 不生成 retract_all", () => {
    const rows = [
      { ...entry({ claude: "synced" }), kind: "mcp" as const },
      { ...entry({ claude: "synced" }), kind: "mcp" as const },
    ];
    expect(
      resolveBatchPlatformToggleMode(rows, "claude", "cursor", false),
    ).toBe("activate_all");
  });
});

describe("resolveBatchEntryPlatformAction", () => {
  it("activate_all：已激活项跳过，未激活项下发", () => {
    const linked = entry({ cursor: "linked", agentmanager: "linked" });
    const unlinked = entry({ cursor: "unlinked", agentmanager: "linked" });
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
    const linked = entry({ cursor: "linked", agentmanager: "linked" });
    const unlinked = entry({ cursor: "unlinked", agentmanager: "linked" });
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

  it("retract_all：agents-manager 批量收回平台副本", () => {
    const linked = entry({ agentmanager: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "agentmanager",
        "retract_all",
        cursorView,
      ),
    ).toBe("retract");
  });

  it("retract_all：源视图浏览 agents-manager 时 skill 不批量收回", () => {
    const linked = entry({ agentmanager: "linked" });
    expect(
      resolveBatchEntryPlatformAction(
        linked,
        "agentmanager",
        "retract_all",
        sourceView,
      ),
    ).toBe("skip");
  });

  it("retract_all：mcp synced 在其它平台不触发收回", () => {
    const synced = { ...entry({ claude: "synced" }), kind: "mcp" as const };
    expect(
      resolveBatchEntryPlatformAction(
        synced,
        "claude",
        "retract_all",
        cursorView,
      ),
    ).toBe("skip");
  });
});
