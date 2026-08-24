import { FolderOpen } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";
import { pickProjectDirectory } from "../../api/tauriAssets";
import { defaultProjectNameFromPath } from "../../utils/defaultProjectNameFromPath";

interface RegisterProjectModalProps {
  busy?: boolean;
  onCancel: () => void;
  onSubmit: (name: string, rootPath: string) => void;
}

export function RegisterProjectModal({
  busy = false,
  onCancel,
  onSubmit,
}: RegisterProjectModalProps) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [pickError, setPickError] = useState<string | null>(null);

  const derivedName = useMemo(
    () => defaultProjectNameFromPath(rootPath),
    [rootPath],
  );
  const effectiveName = name.trim() || derivedName;
  const canSubmit = rootPath.trim().length > 0 && effectiveName.length > 0 && !busy;

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    onSubmit(name.trim(), rootPath.trim());
  };

  const handlePickFolder = async () => {
    if (busy) return;
    setPickError(null);
    try {
      const picked = await pickProjectDirectory(t("registerProject.pickFolderTitle"));
      if (picked) setRootPath(picked);
    } catch (err) {
      setPickError(String(err));
    }
  };

  return (
    <AnimatedOverlay open className="confirm-backdrop" onClick={busy ? undefined : onCancel}>
      <AnimatedScaleDialog
        className="confirm-dialog register-project-dialog"
        aria-labelledby="register-project-title"
      >
        <h4 id="register-project-title">{t("registerProject.title")}</h4>
        <form onSubmit={handleSubmit}>
          <div className="form-field">
            <label htmlFor="register-project-root">{t("registerProject.rootLabel")}</label>
            <div className="path-input-row">
              <input
                id="register-project-root"
                type="text"
                className="path-input"
                value={rootPath}
                disabled={busy}
                autoFocus
                placeholder={t("registerProject.rootPlaceholder")}
                onChange={(e) => setRootPath(e.target.value)}
              />
              <Button
                type="button"
                variant="outline"
                size="icon-sm"
                disabled={busy}
                title={t("registerProject.pickFolderTitle")}
                aria-label={t("registerProject.pickFolder")}
                onClick={() => void handlePickFolder()}
                className="path-input-btn"
              >
                <FolderOpen aria-hidden />
              </Button>
            </div>
            <p className="form-hint">{t("registerProject.rootHint")}</p>
            {pickError ? <p className="form-error">{pickError}</p> : null}
          </div>
          <div className="form-field">
            <label htmlFor="register-project-name">{t("registerProject.nameLabel")}</label>
            <input
              id="register-project-name"
              type="text"
              value={name}
              disabled={busy}
              placeholder={t("registerProject.namePlaceholder")}
              onChange={(e) => setName(e.target.value)}
            />
            {!name.trim() && derivedName ? (
              <p className="form-hint">{t("registerProject.nameDerivedHint", { name: derivedName })}</p>
            ) : (
              <p className="form-hint">{t("registerProject.nameHint")}</p>
            )}
          </div>
          <div className="confirm-actions">
            <button type="button" disabled={busy} onClick={onCancel}>
              {t("confirm.cancel")}
            </button>
            <button type="submit" className="primary" disabled={!canSubmit}>
              {busy ? t("confirm.processing") : t("registerProject.submit")}
            </button>
          </div>
        </form>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
