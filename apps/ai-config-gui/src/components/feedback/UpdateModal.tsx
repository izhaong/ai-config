import { useTranslation } from "react-i18next";

import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";

interface UpdateModalProps {
  open: boolean;
  currentVersion: string;
  newVersion: string;
  notes?: string;
  installing: boolean;
  progress: number | null;
  onInstall: () => void;
  onLater: () => void;
}

export function UpdateModal({
  open,
  currentVersion,
  newVersion,
  notes,
  installing,
  progress,
  onInstall,
  onLater,
}: UpdateModalProps) {
  const { t } = useTranslation();

  return (
    <AnimatedOverlay open={open} className="confirm-backdrop" onClick={onLater}>
      <AnimatedScaleDialog className="confirm-dialog update-dialog">
        <h4>{t("updater.title")}</h4>
        <p>
          {t("updater.message", { current: currentVersion, latest: newVersion })}
        </p>
        {notes ? <p className="update-notes">{notes}</p> : null}
        {installing && progress !== null ? (
          <div className="update-progress" aria-live="polite">
            <div
              className="update-progress-bar"
              style={{ width: `${progress}%` }}
            />
            <span className="update-progress-label">
              {t("updater.downloading", { percent: progress })}
            </span>
          </div>
        ) : null}
        <div className="confirm-actions">
          <button type="button" disabled={installing} onClick={onLater}>
            {t("updater.later")}
          </button>
          <button
            type="button"
            className="primary"
            disabled={installing}
            onClick={onInstall}
          >
            {installing ? t("updater.installing") : t("updater.installNow")}
          </button>
        </div>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
