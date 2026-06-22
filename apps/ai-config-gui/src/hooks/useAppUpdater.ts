import { useMemoizedFn, useMount } from "ahooks";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { ToastKind } from "./useToast";

interface UseAppUpdaterOptions {
  showToast: (kind: ToastKind, text: string) => void;
}

export function useAppUpdater({ showToast }: UseAppUpdaterOptions) {
  const { t } = useTranslation();
  const [update, setUpdate] = useState<Update | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const checkingRef = useRef(false);

  const checkForUpdate = useMemoizedFn(async (manual = false) => {
    if (import.meta.env.DEV || checkingRef.current) {
      return;
    }
    checkingRef.current = true;
    try {
      const found = await check();
      if (found) {
        setUpdate(found);
        setModalOpen(true);
        return;
      }
      if (manual) {
        showToast("ok", t("updater.toast.alreadyLatest"));
      }
    } catch (err) {
      if (manual) {
        showToast(
          "err",
          t("updater.toast.checkFailed", { error: String(err) }),
        );
      }
    } finally {
      checkingRef.current = false;
    }
  });

  useMount(() => {
    void checkForUpdate(false);
  });

  const dismissModal = useMemoizedFn(() => {
    if (!installing) {
      setModalOpen(false);
    }
  });

  const installUpdate = useMemoizedFn(async () => {
    if (!update) {
      return;
    }
    setInstalling(true);
    setProgress(0);
    let downloaded = 0;
    let contentLength = 0;
    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            contentLength = event.data.contentLength ?? 0;
            downloaded = 0;
            setProgress(0);
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            if (contentLength > 0) {
              setProgress(
                Math.min(100, Math.round((downloaded / contentLength) * 100)),
              );
            }
            break;
          case "Finished":
            setProgress(100);
            break;
        }
      });
      await relaunch();
    } catch (err) {
      showToast(
        "err",
        t("updater.toast.installFailed", { error: String(err) }),
      );
      setInstalling(false);
      setProgress(null);
    }
  });

  return {
    update,
    modalOpen,
    installing,
    progress,
    dismissModal,
    installUpdate,
    checkForUpdate,
  };
}
