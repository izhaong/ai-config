import { describe, expect, it } from "vitest";

import { sanitizeListDescription } from "./sanitizeListDescription";

describe("sanitizeListDescription", () => {
  it("passes through normal text", () => {
    expect(sanitizeListDescription("zh-cloud Web frontend expert")).toBe(
      "zh-cloud Web frontend expert",
    );
  });

  it("strips description: >- prefix artifact", () => {
    expect(sanitizeListDescription("description: >-")).toBe("");
    expect(sanitizeListDescription("description: >- 跨端 UX 默认偏好")).toBe(
      "跨端 UX 默认偏好",
    );
  });

  it("strips standalone block scalar indicators", () => {
    expect(sanitizeListDescription(">-")).toBe("");
    expect(sanitizeListDescription("|+")).toBe("");
  });

  it("strips leading/trailing block indicators", () => {
    expect(sanitizeListDescription(">- Ant Design skill")).toBe(
      "Ant Design skill",
    );
    expect(sanitizeListDescription("Ant Design skill >-")).toBe(
      "Ant Design skill",
    );
  });

  it("collapses whitespace for list display", () => {
    expect(sanitizeListDescription("  第一段   第二段  ")).toBe(
      "第一段 第二段",
    );
  });
});
