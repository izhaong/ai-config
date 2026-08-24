import { useTranslation } from "react-i18next";

import type { ImportPlan } from "../../types";
import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";

interface ImportPlanModalProps {
  open: boolean;
  plan: ImportPlan | null;
  busy?: boolean;
  onApply: () => void;
  onCancel: () => void;
}

/** Explicit platform-to-source import review; platform content never enters this payload. */
export function ImportPlanModal({
  open,
  plan,
  busy = false,
  onApply,
  onCancel,
}: ImportPlanModalProps) {
  const { t } = useTranslation();
  if (!plan) return null;
  const action = plan.actions[0];
  const blocked = !action || action.blocking_reasons.length > 0 || action.secret_preflight.status === "blocked";

  return (
    <AnimatedOverlay open={open} className="confirm-backdrop" onClick={onCancel}>
      <AnimatedScaleDialog className="confirm-dialog projection-plan-dialog">
        <h4>{t("projection.importTitle")}</h4>
        <p>{t("projection.importMessage")}</p>
        {action ? (
          <ul className="projection-plan-actions">
            <li>
              <strong>{action.normalized_diff.change}</strong>
              <code>{action.source_path}</code>
              <small>{action.destination_path}</small>
              {action.secret_preflight.key_names.length > 0 ? (
                <small>{t("projection.missingSecrets", { keys: action.secret_preflight.key_names.join(", ") })}</small>
              ) : null}
              {action.blocking_reasons.map((reason) => <small key={reason}>{reason}</small>)}
            </li>
          </ul>
        ) : null}
        <div className="confirm-actions">
          <button type="button" disabled={busy} onClick={onCancel}>{t("confirm.cancel")}</button>
          <button type="button" disabled={busy || blocked} onClick={onApply}>
            {busy ? t("confirm.processing") : t("projection.confirmImport")}
          </button>
        </div>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
