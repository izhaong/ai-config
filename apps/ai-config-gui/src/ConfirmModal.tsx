import { useTranslation } from "react-i18next";

interface ConfirmModalProps {
  title: string;
  message: string;
  confirmLabel?: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmModal({
  title,
  message,
  confirmLabel,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
  const { t } = useTranslation();
  const confirm = confirmLabel ?? t("confirm.defaultConfirm");

  return (
    <div className="confirm-backdrop" onClick={onCancel}>
      <div
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <h4>{title}</h4>
        <p>{message}</p>
        <div className="confirm-actions">
          <button type="button" disabled={busy} onClick={onCancel}>
            {t("confirm.cancel")}
          </button>
          <button
            type="button"
            className="danger"
            disabled={busy}
            onClick={onConfirm}
          >
            {busy ? t("confirm.processing") : confirm}
          </button>
        </div>
      </div>
    </div>
  );
}
