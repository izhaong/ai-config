import { describe, expect, it } from "vitest";

import { canDeploy, canRemoveFromPlatform } from "./types";

describe("canRemoveFromPlatform", () => {
  it("allows only managed non-MCP assets", () => {
    expect(canRemoveFromPlatform("skill", "synced")).toBe(false);
    expect(canRemoveFromPlatform("skill", "linked")).toBe(true);
    expect(canRemoveFromPlatform("mcp", "linked")).toBe(false);
  });
});

describe("canDeploy", () => {
  it("does not implicitly adopt a synced platform copy", () => {
    expect(canDeploy("synced")).toBe(false);
  });
});
