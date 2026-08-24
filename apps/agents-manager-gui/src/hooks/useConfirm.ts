import { useMemoizedFn } from "ahooks";
import { useState } from "react";

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel?: string;
  onConfirm: () => Promise<void>;
}

export function useConfirm() {
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

  const requestConfirm = useMemoizedFn((req: ConfirmRequest) => {
    setConfirm(req);
  });

  const dismissConfirm = useMemoizedFn(() => setConfirm(null));

  return { confirm, requestConfirm, dismissConfirm, setConfirm };
}
