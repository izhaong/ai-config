import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

interface AddSkillModalProps {
  busy?: boolean;
  platformLabel: string;
  onCancel: () => void;
  onSubmit: (source: string, skillName?: string) => void;
}

export function AddSkillModal({
  busy = false,
  platformLabel,
  onCancel,
  onSubmit,
}: AddSkillModalProps) {
  const { t } = useTranslation();
  const [source, setSource] = useState("");
  const [skillName, setSkillName] = useState("");

  const canSubmit = source.trim().length > 0 && !busy;

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    const trimmedName = skillName.trim();
    onSubmit(source.trim(), trimmedName || undefined);
  };

  return (
    <div className="confirm-backdrop" onClick={busy ? undefined : onCancel}>
      <div
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-skill-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h4 id="add-skill-title">{t("addSkill.title")}</h4>
        <p className="form-hint add-skill-target">
          {t("addSkill.targetHint", { platform: platformLabel })}
        </p>
        <form onSubmit={handleSubmit}>
          <div className="form-field">
            <label htmlFor="add-skill-source">{t("addSkill.sourceLabel")}</label>
            <input
              id="add-skill-source"
              type="text"
              value={source}
              disabled={busy}
              autoFocus
              placeholder="vercel-labs/agent-skills"
              onChange={(e) => setSource(e.target.value)}
            />
            <p className="form-hint">{t("addSkill.sourceHint")}</p>
          </div>
          <div className="form-field">
            <label htmlFor="add-skill-name">{t("addSkill.skillNameLabel")}</label>
            <input
              id="add-skill-name"
              type="text"
              value={skillName}
              disabled={busy}
              placeholder={t("addSkill.skillNamePlaceholder")}
              onChange={(e) => setSkillName(e.target.value)}
            />
            <p className="form-hint">{t("addSkill.skillNameHint")}</p>
          </div>
          <div className="confirm-actions">
            <button type="button" disabled={busy} onClick={onCancel}>
              {t("confirm.cancel")}
            </button>
            <button type="submit" className="primary" disabled={!canSubmit}>
              {busy ? t("confirm.processing") : t("addSkill.submit")}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
