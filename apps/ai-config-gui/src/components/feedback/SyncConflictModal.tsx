import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";
import { PLATFORM_NAME } from "../../platformIcons";
import type { Platform, SyncConflictReport } from "../../types";

interface SyncConflictModalProps {
  open: boolean;
  report: SyncConflictReport | null;
  defaultSource: Platform;
  busy?: boolean;
  onConfirm: (sourcePlatform: Platform) => void;
  onCancel: () => void;
}

export function SyncConflictModal({
  open,
  report,
  defaultSource,
  busy = false,
  onConfirm,
  onCancel,
}: SyncConflictModalProps) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Platform | null>(null);

  const source = selected ?? defaultSource;
  const targetLabel = report
    ? PLATFORM_NAME[report.target_platform]
    : "";

  if (!report) {
    return null;
  }

  return (
    <AnimatedOverlay open={open} className="confirm-backdrop" onClick={onCancel}>
      <AnimatedScaleDialog className="confirm-dialog sync-conflict-dialog">
        <h4>{t("syncConflict.title", { name: report.name })}</h4>
        <p>
          {t("syncConflict.message", {
            baseline: PLATFORM_NAME[report.baseline_platform],
            target: targetLabel,
          })}
        </p>

        <div className="sync-conflict-variants" role="radiogroup">
          {report.variants.map((variant) => (
            <label
              key={variant.platform}
              className={`sync-conflict-variant${
                source === variant.platform ? " sync-conflict-variant--selected" : ""
              }`}
            >
              <input
                type="radio"
                name="sync-source"
                value={variant.platform}
                checked={source === variant.platform}
                disabled={busy}
                onChange={() => setSelected(variant.platform)}
              />
              <div className="sync-conflict-variant-head">
                <strong>{PLATFORM_NAME[variant.platform]}</strong>
                {variant.differs_from_baseline ? (
                  <span className="sync-conflict-badge">
                    {t("syncConflict.differs")}
                  </span>
                ) : (
                  <span className="sync-conflict-badge sync-conflict-badge--baseline">
                    {t("syncConflict.baseline")}
                  </span>
                )}
                {variant.content_summary ? (
                  <span className="sync-conflict-summary">
                    {variant.content_summary}
                  </span>
                ) : null}
              </div>
              <pre className="sync-conflict-preview">{variant.content}</pre>
            </label>
          ))}
        </div>

        <div className="confirm-actions">
          <button type="button" disabled={busy} onClick={onCancel}>
            {t("confirm.cancel")}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => onConfirm(source)}
          >
            {busy
              ? t("confirm.processing")
              : t("syncConflict.confirm", { target: targetLabel })}
          </button>
        </div>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
