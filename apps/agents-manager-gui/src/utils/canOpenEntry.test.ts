import { describe, expect, it } from "vitest";
import type { Platform, PlatformAssetEntry } from "../types";
import { canOpenEntry, shouldLoadFromSource } from "./canOpenEntry";

const ALL_STATES = (
  overrides: Partial<
    Record<Platform, PlatformAssetEntry["states"][Platform]>
  > = {},
): PlatformAssetEntry["states"] => ({
  agentsmanager: "unlinked",
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
    expect(canOpenEntry(e)).toBe(true);
  });

  it("opens when agents-manager source is linked", () => {
    const e = entry({ states: ALL_STATES({ agentsmanager: "linked" }) });
    expect(canOpenEntry(e)).toBe(true);
  });

  it("opens mcp on agents-manager source view", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.agents/mcp.json",
    });
    expect(canOpenEntry(e)).toBe(true);
  });

  it("opens mcp from deploy platform when platform_path exists", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.cursor/mcp.json",
      states: ALL_STATES({ cursor: "synced" }),
    });
    expect(canOpenEntry(e)).toBe(true);
  });

  it("does not open without platform_path", () => {
    const e = entry({ platform_path: "" });
    expect(canOpenEntry(e)).toBe(false);
  });
});

describe("shouldLoadFromSource", () => {
  it("loads from source on agents-manager view for skills", () => {
    const e = entry();
    expect(shouldLoadFromSource(e, "agentsmanager")).toBe(true);
  });

  it("loads from source when agentsmanager linked on deploy view", () => {
    const e = entry({ states: ALL_STATES({ agentsmanager: "linked" }) });
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

  it("mcp with agentsmanager linked loads from source on deploy view", () => {
    const e = entry({
      kind: "mcp",
      platform_path: "/home/.agents/mcp.json",
      states: ALL_STATES({ agentsmanager: "linked", cursor: "linked" }),
    });
    expect(shouldLoadFromSource(e, "cursor")).toBe(true);
  });
});
