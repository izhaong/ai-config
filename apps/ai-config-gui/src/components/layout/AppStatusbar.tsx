import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { DoctorSummary } from "../../types";
import { SettingsMenu } from "./SettingsMenu";

interface AppStatusbarProps {
  doctor: DoctorSummary | null;
  doctorIssueCount: number;
  projectCount: number;
  loading?: boolean;
  busy?: boolean;
  onRefresh?: () => void;
  gitStatusLabel?: string;
  gitSettings: {
    assetRoot: string;
    remoteDraft: string;
    branchDraft: string;
    onRemoteChange: (value: string) => void;
    onBranchChange: (value: string) => void;
    onSaveGitConfig: () => void;
    onGitSync: () => void;
    onGitPull: () => void;
    onGitPush: () => void;
    gitBusy: boolean;
    hasRemote: boolean;
  };
}

export function AppStatusbar({
  doctor,
  doctorIssueCount,
  projectCount,
  loading = false,
  busy = false,
  onRefresh,
  gitStatusLabel,
  gitSettings,
}: AppStatusbarProps) {
  const { t } = useTranslation();

  return (
    <footer className="statusbar">
      <span className="statusbar-app">{t("app.name")}</span>
      <span className="statusbar-version">{t("app.version")}</span>
      <span className="statusbar-daemon">
        <span className="status-dot status-dot-down" aria-hidden />
        {t("app.daemonStopped")}
      </span>

      <span className="statusbar-divider" aria-hidden />

      <span className="statusbar-stat">
        {doctor
          ? t("statusbar.doctor", { count: doctorIssueCount })
          : t("statusbar.doctorLoading")}
      </span>
      <span className="statusbar-stat">
        {t("statusbar.secrets", { count: doctor?.missing_secrets.length ?? 0 })}
      </span>
      <span className="statusbar-stat">
        {t("statusbar.projects", { count: projectCount })}
      </span>
      <span className="statusbar-stat">
        {t("statusbar.broken", { count: 0 })}
      </span>
      {gitStatusLabel ? (
        <span className="statusbar-stat statusbar-git" title={gitSettings.assetRoot}>
          git: {gitStatusLabel}
        </span>
      ) : null}

      <div className="statusbar-right">
        {onRefresh ? (
          <button
            type="button"
            className="statusbar-icon-btn"
            disabled={loading || busy}
            title={
              loading ? t("toolbar.refreshing") : t("toolbar.refreshTitle")
            }
            aria-label={t("toolbar.refresh")}
            onClick={onRefresh}
          >
            <RefreshCw size={14} className={loading ? "spin" : undefined} />
          </button>
        ) : null}
        <SettingsMenu
          assetRoot={gitSettings.assetRoot}
          remoteDraft={gitSettings.remoteDraft}
          branchDraft={gitSettings.branchDraft}
          onRemoteChange={gitSettings.onRemoteChange}
          onBranchChange={gitSettings.onBranchChange}
          onSaveGitConfig={gitSettings.onSaveGitConfig}
          onGitSync={gitSettings.onGitSync}
          onGitPull={gitSettings.onGitPull}
          onGitPush={gitSettings.onGitPush}
          gitBusy={gitSettings.gitBusy}
          gitStatusLabel={gitStatusLabel}
          hasRemote={gitSettings.hasRemote}
        />
      </div>
    </footer>
  );
}
