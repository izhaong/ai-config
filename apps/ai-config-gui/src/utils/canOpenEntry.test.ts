import { describe, expect, it } from "vitest";
import type { Platform, PlatformAssetEntry } from "../types";
import { canOpenEntry, shouldLoadFromSource } from "./canOpenEntry";

const ALL_STATES = (
  overrides: Partial<
    Record<Platform, PlatformAssetEntry["states"][Platform]>
  > = {},
): PlatformAssetEntry["states"] => ({
  aiconfig: "unlinked",
  cursor: "unlinked",
  codex: "unlinked",
  claude: "unlinked",
  hermes: "unlinked",
  ...overrides,
});

function entry(
  overrides: Partial<PlatformAssetEntry> = {},
): PlatformAssetEntry {
  return {
    name: "demo",
    kind: "skill",
    description: "",
    platform_path: "/tmp/skills/demo/SKILL.md",
    states: ALL_STATES(),
    ...overrides,
  };
}

describe("canOpenEntry", () => {
  it("opens when platform_path exists (deploy view)", () => {
    const e = entry();
    expect(canOpenEntry(e, "cursor")).toBe(true);
  });

  it("opens when ai-config source is linked", () => {
    const e = entry({ states: ALL_STATES({ aiconfig: "linked" }) });
    expect(canOpenEntry(e, "cursor")).toBe(true);
  });

  it("opens mcp on ai-config source view", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.ai-config/mcp.json",
    });
    expect(canOpenEntry(e, "aiconfig")).toBe(true);
  });

  it("opens mcp from deploy platform when platform_path exists", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.cursor/mcp.json",
      states: ALL_STATES({ cursor: "synced" }),
    });
    expect(canOpenEntry(e, "cursor")).toBe(true);
  });

  it("does not open without platform_path", () => {
    const e = entry({ platform_path: "" });
    expect(canOpenEntry(e, "cursor")).toBe(false);
  });
});

describe("shouldLoadFromSource", () => {
  it("loads from source on ai-config view for skills", () => {
    const e = entry();
    expect(shouldLoadFromSource(e, "aiconfig")).toBe(true);
  });

  it("loads from source when aiconfig linked on deploy view", () => {
    const e = entry({ states: ALL_STATES({ aiconfig: "linked" }) });
    expect(shouldLoadFromSource(e, "cursor")).toBe(true);
  });

  it("previews from platform path when only deploy linked", () => {
    const e = entry({ states: ALL_STATES({ cursor: "linked" }) });
    expect(shouldLoadFromSource(e, "cursor")).toBe(false);
  });

  it("mcp without source previews from platform on deploy view", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.cursor/mcp.json",
      states: ALL_STATES({ cursor: "synced" }),
    });
    expect(shouldLoadFromSource(e, "cursor")).toBe(false);
  });

  it("mcp with aiconfig linked loads from source on deploy view", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.ai-config/mcp.json",
      states: ALL_STATES({ aiconfig: "linked", cursor: "linked" }),
    });
    expect(shouldLoadFromSource(e, "cursor")).toBe(true);
  });
});
