import { useTranslation } from "react-i18next";

import type { ProjectionReview } from "../../types";
import { canApplyProjectionReview } from "../../utils/projectionReview";
import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";

interface ProjectionPlanModalProps {
  open: boolean;
  review: ProjectionReview | null;
  retract: boolean;
  busy?: boolean;
  onApply: () => void;
  onCancel: () => void;
}

/** The only GUI confirmation surface for source-first projection writes. */
export function ProjectionPlanModal({
  open,
  review,
  retract,
  busy = false,
  onApply,
  onCancel,
}: ProjectionPlanModalProps) {
  const { t } = useTranslation();
  if (!review) return null;
  const canApply = canApplyProjectionReview(review);

  return (
    <AnimatedOverlay open={open} className="confirm-backdrop" onClick={onCancel}>
      <AnimatedScaleDialog className="confirm-dialog projection-plan-dialog">
        <h4>{retract ? t("projection.retractTitle") : t("projection.applyTitle")}</h4>
        <p>{t("projection.reviewMessage")}</p>
        <div className="projection-plan-digest">{review.plan_digest}</div>
        {review.ledger_status ? (
          <p className="sync-conflict-badge">{t("projection.ledgerUnavailable")}</p>
        ) : null}
        {review.blocking_reason ? (
          <p className="sync-conflict-badge">{t("projection.blocked", { reason: review.blocking_reason })}</p>
        ) : null}
        <ul className="projection-plan-actions">
          {review.actions.map((action) => (
            <li key={action.action_id}>
              <strong>{action.kind}</strong>
              <span>{action.reason_code}</span>
              {action.target?.path ? <code>{action.target.path}</code> : null}
              {action.members[0]?.source.absolute_path ? (
                <small>{action.members[0].source.absolute_path}</small>
              ) : null}
              {action.mcp_members.flatMap((member) => member.missing_secret_keys).length > 0 ? (
                <small>
                  {t("projection.missingSecrets", {
                    keys: review.actions
                      .flatMap((item) => item.mcp_members)
                      .flatMap((member) => member.missing_secret_keys)
                      .join(", "),
                  })}
                </small>
              ) : null}
            </li>
          ))}
        </ul>
        <div className="confirm-actions">
          <button type="button" disabled={busy} onClick={onCancel}>
            {t("confirm.cancel")}
          </button>
          <button type="button" disabled={busy || !canApply} onClick={onApply}>
            {busy ? t("confirm.processing") : t("projection.confirm")}
          </button>
        </div>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
