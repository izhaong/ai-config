import { useTranslation } from "react-i18next";

import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";

interface ConfirmModalProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmModal({
  open,
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
    <AnimatedOverlay open={open} className="confirm-backdrop" onClick={onCancel}>
      <AnimatedScaleDialog className="confirm-dialog">
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
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
