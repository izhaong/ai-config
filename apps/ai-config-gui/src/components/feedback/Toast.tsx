import type { ToastState } from "../../hooks/useToast";

interface ToastProps {
  toast: ToastState;
  onDismiss: () => void;
}

export function Toast({ toast, onDismiss }: ToastProps) {
  if (!toast) return null;

  return (
    <div className={`toast ${toast.kind}`} onClick={onDismiss}>
      {toast.text}
    </div>
  );
}
