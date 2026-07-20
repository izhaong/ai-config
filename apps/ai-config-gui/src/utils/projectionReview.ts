import type { ProjectionReview } from "../types";

/** A review is executable only through its current digest and a readable ownership ledger. */
export function canApplyProjectionReview(review: ProjectionReview): boolean {
  return Boolean(
    review.plan_digest &&
      !review.blocking_reason &&
      review.ledger_status !== "ledger_unavailable",
  );
}
