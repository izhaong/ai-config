import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

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

  useEffect(() => {
    setName("");
    setRootPath("");
  }, []);

  const canSubmit = name.trim().length > 0 && rootPath.trim().length > 0 && !busy;

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    onSubmit(name.trim(), rootPath.trim());
  };

  return (
    <div className="confirm-backdrop" onClick={busy ? undefined : onCancel}>
      <div
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="register-project-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h4 id="register-project-title">{t("registerProject.title")}</h4>
        <form onSubmit={handleSubmit}>
          <div className="form-field">
            <label htmlFor="register-project-name">{t("registerProject.nameLabel")}</label>
            <input
              id="register-project-name"
              type="text"
              value={name}
              disabled={busy}
              autoFocus
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div className="form-field">
            <label htmlFor="register-project-root">{t("registerProject.rootLabel")}</label>
            <input
              id="register-project-root"
              type="text"
              value={rootPath}
              disabled={busy}
              placeholder="/path/to/repo"
              onChange={(e) => setRootPath(e.target.value)}
            />
            <p className="form-hint">{t("registerProject.rootHint")}</p>
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
      </div>
    </div>
  );
}
