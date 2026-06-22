import { AnimatePresence, motion } from "motion/react";

import { useMotionPresets } from "@/hooks/useMotionPresets";
import type { ToastState } from "../../hooks/useToast";

interface ToastProps {
  toast: ToastState;
  onDismiss: () => void;
}

export function Toast({ toast, onDismiss }: ToastProps) {
  const { toast: toastMotion } = useMotionPresets();

  return (
    <AnimatePresence mode="wait">
      {toast ? (
        <motion.div
          key={toast.text}
          className={`toast ${toast.kind}`}
          onClick={onDismiss}
          initial={toastMotion.initial}
          animate={toastMotion.animate}
          exit={toastMotion.exit}
          transition={toastMotion.transition}
        >
          {toast.text}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
