import { describe, expect, it } from "vitest";

import type { ProjectionReview } from "../types";
import { canApplyProjectionReview } from "./projectionReview";

const review = (overrides: Partial<ProjectionReview> = {}): ProjectionReview => ({
  schema_version: 1,
  plan_digest: "digest",
  actions: [],
  blocking_reason: null,
  ...overrides,
});

describe("canApplyProjectionReview", () => {
  it("只允许可写 ledger 且没有阻塞项的 digest-bound review 执行", () => {
    expect(canApplyProjectionReview(review())).toBe(true);
    expect(
      canApplyProjectionReview(review({ blocking_reason: "foreign_target" })),
    ).toBe(false);
    expect(
      canApplyProjectionReview(review({ ledger_status: "ledger_unavailable" })),
    ).toBe(false);
    expect(canApplyProjectionReview(review({ plan_digest: "" }))).toBe(false);
  });
});
