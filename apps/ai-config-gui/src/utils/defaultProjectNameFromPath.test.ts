import { describe, expect, it } from "vitest";

import { defaultProjectNameFromPath } from "./defaultProjectNameFromPath";

describe("defaultProjectNameFromPath", () => {
  it("returns last path segment", () => {
    expect(defaultProjectNameFromPath("/Users/me/Code/my-repo")).toBe(
      "my-repo",
    );
    expect(defaultProjectNameFromPath("C:\\dev\\agent-manager\\")).toBe(
      "agent-manager",
    );
  });

  it("returns empty for blank path", () => {
    expect(defaultProjectNameFromPath("")).toBe("");
    expect(defaultProjectNameFromPath("   ")).toBe("");
  });
});
