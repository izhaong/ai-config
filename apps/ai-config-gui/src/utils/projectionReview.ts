import type { ProjectionReview } from "../types";

/** A review is executable only through its current digest and a readable ownership ledger. */
export function canApplyProjectionReview(review: ProjectionReview): boolean {
  const hasAdoption = review.actions.some(
    (action) => action.kind === "adopt_equivalent",
  );
  return Boolean(
    review.plan_digest &&
      (!review.blocking_reason || hasAdoption) &&
      review.ledger_status !== "ledger_unavailable",
  );
}
