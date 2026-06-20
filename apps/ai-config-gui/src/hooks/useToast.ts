import { useMemoizedFn, useTimeout } from "ahooks";
import { useState } from "react";

export type ToastKind = "err" | "ok";
export type ToastState = { kind: ToastKind; text: string } | null;

export function useToast() {
  const [toast, setToast] = useState<ToastState>(null);

  const showToast = useMemoizedFn((kind: ToastKind, text: string) => {
    setToast({ kind, text });
  });

  const dismissToast = useMemoizedFn(() => setToast(null));

  useTimeout(dismissToast, toast ? 4000 : undefined);

  return { toast, showToast, dismissToast, setToast };
}
