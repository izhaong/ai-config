import { describe, expect, it } from "vitest";

import { rowSyncLeadSlot } from "./rowSyncLeadSlot";

describe("rowSyncLeadSlot", () => {
  it("skill always reserves lead menu column", () => {
    expect(rowSyncLeadSlot("skill", false)).toBe("skill-menu");
    expect(rowSyncLeadSlot("skill", true)).toBe("skill-menu");
  });

  it("mcp add only on source view", () => {
    expect(rowSyncLeadSlot("mcp", true)).toBe("mcp-add");
    expect(rowSyncLeadSlot("mcp", false)).toBe("none");
  });
});
