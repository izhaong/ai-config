import { useClickAway } from "ahooks";
import { GitBranch, Settings } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { LanguageSwitcher } from "./LanguageSwitcher";

interface SettingsMenuProps {
  assetRoot?: string;
  remoteDraft: string;
  branchDraft: string;
  onRemoteChange: (value: string) => void;
  onBranchChange: (value: string) => void;
  onSaveGitConfig: () => void;
  onGitSync: () => void;
  onGitPull: () => void;
  onGitPush: () => void;
  gitBusy?: boolean;
  gitStatusLabel?: string;
  hasRemote?: boolean;
}

export function SettingsMenu({
  assetRoot,
  remoteDraft,
  branchDraft,
  onRemoteChange,
  onBranchChange,
  onSaveGitConfig,
  onGitSync,
  onGitPull,
  onGitPush,
  gitBusy = false,
  gitStatusLabel,
  hasRemote = false,
}: SettingsMenuProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useClickAway(() => setOpen(false), rootRef);

  return (
    <div className="settings-menu" ref={rootRef}>
      <button
        type="button"
        className="settings-menu-trigger"
        aria-haspopup="menu"
        aria-expanded={open}
        title={t("settings.title")}
        onClick={() => setOpen((v) => !v)}
      >
        <Settings size={14} aria-hidden />
        <span>{t("settings.title")}</span>
      </button>
      {open ? (
        <div className="settings-menu-panel settings-menu-panel-wide" role="menu">
          <div className="settings-menu-section">
            <span className="settings-menu-label">{t("git.settings.title")}</span>
            {assetRoot ? (
              <span className="settings-menu-hint" title={assetRoot}>
                {t("git.settings.assetRoot", { path: assetRoot })}
              </span>
            ) : null}
            {gitStatusLabel ? (
              <span className="settings-menu-git-status">
                <GitBranch size={12} aria-hidden />
                {gitStatusLabel}
              </span>
            ) : null}
            <label className="settings-menu-field">
              <span>{t("git.settings.remoteUrl")}</span>
              <input
                type="url"
                value={remoteDraft}
                disabled={gitBusy}
                placeholder={t("git.settings.remotePlaceholder")}
                onChange={(e) => onRemoteChange(e.target.value)}
              />
            </label>
            <label className="settings-menu-field">
              <span>{t("git.settings.branch")}</span>
              <input
                type="text"
                value={branchDraft}
                disabled={gitBusy}
                placeholder="main"
                onChange={(e) => onBranchChange(e.target.value)}
              />
            </label>
            <p className="settings-menu-hint">{t("git.settings.localHint")}</p>
            <div className="settings-menu-actions">
              <button
                type="button"
                className="settings-menu-btn"
                disabled={gitBusy}
                onClick={onSaveGitConfig}
              >
                {t("git.settings.save")}
              </button>
              <button
                type="button"
                className="settings-menu-btn settings-menu-btn-primary"
                disabled={gitBusy}
                onClick={onGitSync}
              >
                {t("git.settings.sync")}
              </button>
            </div>
            <div className="settings-menu-actions">
              <button
                type="button"
                className="settings-menu-btn"
                disabled={gitBusy || !hasRemote}
                title={hasRemote ? undefined : t("git.settings.remoteRequired")}
                onClick={onGitPull}
              >
                {t("git.settings.pull")}
              </button>
              <button
                type="button"
                className="settings-menu-btn"
                disabled={gitBusy || !hasRemote}
                title={hasRemote ? undefined : t("git.settings.remoteRequired")}
                onClick={onGitPush}
              >
                {t("git.settings.push")}
              </button>
            </div>
          </div>
          <div className="settings-menu-divider" />
          <div className="settings-menu-section">
            <span className="settings-menu-label">{t("language.switch")}</span>
            <LanguageSwitcher inline />
          </div>
        </div>
      ) : null}
    </div>
  );
}
