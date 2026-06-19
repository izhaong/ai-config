import { useLatest, useMemoizedFn } from "ahooks";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  deleteAsset as apiDeleteAsset,
  fetchAsset,
  readPlatformAsset,
  saveAsset as apiSaveAsset,
} from "../api/tauriAssets";
import { assetKindLabel } from "../i18n/labels";
import type { ConfirmRequest } from "./useConfirm";
import type { ToastKind } from "./useToast";
import type {
  AssetDetail,
  AssetKind,
  Platform,
  PlatformAssetEntry,
  PlatformAssetList,
} from "../types";
import {
  canOpenEntry as canOpenAssetEntry,
  shouldLoadFromSource,
} from "../utils/canOpenEntry";

interface UseAssetDrawerOptions {
  activeProject: string;
  activeKind: AssetKind;
  activePlatform: Platform;
  visible: PlatformAssetEntry[];
  browsingSource: boolean;
  refresh: () => Promise<PlatformAssetList | null>;
  clearSelection: () => void;
  showToast: (kind: ToastKind, text: string) => void;
  requestConfirm: (req: ConfirmRequest) => void;
  dismissConfirm: () => void;
  setBusy: (busy: boolean) => void;
}

export function useAssetDrawer({
  activeProject,
  activeKind,
  activePlatform,
  visible,
  browsingSource,
  refresh,
  clearSelection,
  showToast,
  requestConfirm,
  dismissConfirm,
  setBusy,
}: UseAssetDrawerOptions) {
  const { t } = useTranslation();
  const [drawerName, setDrawerName] = useState<string | null>(null);
  const [drawerDetail, setDrawerDetail] = useState<AssetDetail | null>(null);
  const [drawerEditing, setDrawerEditing] = useState(false);
  const [drawerDraft, setDrawerDraft] = useState("");
  const [drawerLoading, setDrawerLoading] = useState(false);
  const [drawerReadOnly, setDrawerReadOnly] = useState(false);

  const drawerNameLatest = useLatest(drawerName);
  const drawerEditingLatest = useLatest(drawerEditing);
  const kindLabel = assetKindLabel(t, activeKind);

  const closeDrawer = useMemoizedFn(() => {
    setDrawerName(null);
    setDrawerDetail(null);
    setDrawerEditing(false);
    setDrawerDraft("");
    setDrawerReadOnly(false);
  });

  const resetDrawer = useMemoizedFn(() => {
    setDrawerName(null);
    setDrawerDetail(null);
    setDrawerEditing(false);
    setDrawerDraft("");
    setDrawerReadOnly(false);
  });

  const canOpenEntry = useMemoizedFn((entry: PlatformAssetEntry) =>
    canOpenAssetEntry(entry, activePlatform),
  );

  const reloadDrawerIfOpen = useMemoizedFn(
    async (listResult: PlatformAssetList | null) => {
      const name = drawerNameLatest.current;
      if (!name || !listResult) return;

      const entry = listResult.entries.find(
        (e) => e.kind === activeKind && e.name === name,
      );
      if (!entry || !canOpenEntry(entry)) {
        closeDrawer();
        return;
      }

      const fromSource = shouldLoadFromSource(entry, activePlatform);
      setDrawerReadOnly(!fromSource);

      try {
        const d = fromSource
          ? await fetchAsset(entry.kind, name, activeProject)
          : await readPlatformAsset(
              entry.platform_path,
              entry.kind,
              entry.name,
            );
        setDrawerDetail(d);
        if (!drawerEditingLatest.current) {
          setDrawerDraft(d.content);
        }
      } catch (e) {
        closeDrawer();
        showToast(
          "err",
          t("toast.loadAssetFailed", {
            label: assetKindLabel(t, activeKind),
            error: e,
          }),
        );
      }
    },
  );

  const openAsset = useMemoizedFn(async (entry: PlatformAssetEntry) => {
    const fromSource = shouldLoadFromSource(entry, activePlatform);
    setDrawerName(entry.name);
    setDrawerReadOnly(!fromSource);
    setDrawerEditing(false);
    setDrawerDetail(null);
    setDrawerLoading(true);
    try {
      const d = fromSource
        ? await fetchAsset(entry.kind, entry.name, activeProject)
        : await readPlatformAsset(entry.platform_path, entry.kind, entry.name);
      setDrawerDetail(d);
      setDrawerDraft(d.content);
    } catch (e) {
      showToast(
        "err",
        t("toast.loadAssetFailed", {
          label: assetKindLabel(t, entry.kind),
          error: e,
        }),
      );
      setDrawerName(null);
      setDrawerReadOnly(false);
    } finally {
      setDrawerLoading(false);
    }
  });

  const saveAsset = useMemoizedFn(async () => {
    if (!drawerName || drawerReadOnly) return;
    setDrawerLoading(true);
    try {
      const r = await apiSaveAsset(
        activeKind,
        drawerName,
        drawerDraft,
        activeProject,
      );
      showToast("ok", r);
      setDrawerEditing(false);
      const d = await fetchAsset(activeKind, drawerName, activeProject);
      setDrawerDetail(d);
      setDrawerDraft(d.content);
      await refresh();
    } catch (e) {
      showToast("err", t("toast.saveFailed", { error: e }));
    } finally {
      setDrawerLoading(false);
    }
  });

  const runDeleteEntries = useMemoizedFn(
    async (entries: PlatformAssetEntry[]) => {
      setBusy(true);
      let ok = 0;
      const failed: string[] = [];
      try {
        for (const entry of entries) {
          try {
            await apiDeleteAsset(entry.kind, entry.name, activeProject);
            ok += 1;
          } catch (e) {
            failed.push(`${entry.name}: ${e}`);
          }
        }
        clearSelection();
        if (
          drawerNameLatest.current &&
          entries.some((e) => e.name === drawerNameLatest.current)
        ) {
          closeDrawer();
        }
        await refresh();
        if (failed.length === 0) {
          showToast("ok", t("toast.deletedCount", { count: ok }));
        } else if (ok === 0) {
          showToast(
            "err",
            t("toast.deleteFailed", { errors: failed.join("；") }),
          );
        } else {
          showToast(
            "err",
            t("toast.deletePartial", {
              ok,
              failed: failed.length,
              errors: failed.join("；"),
            }),
          );
        }
      } finally {
        setBusy(false);
        dismissConfirm();
      }
    },
  );

  const requestDeleteAsset = useMemoizedFn(() => {
    if (!drawerName || drawerReadOnly || !browsingSource) return;
    const entry = visible.find((e) => e.name === drawerName);
    if (!entry) {
      showToast("err", t("toast.entryNotFound"));
      return;
    }
    requestConfirm({
      title: t("confirm.deleteSource"),
      message: t("confirm.deleteSourceMessage", {
        label: kindLabel,
        name: drawerName,
        irreversible: t("confirm.irreversible"),
      }),
      confirmLabel: t("confirm.delete"),
      onConfirm: async () => {
        await runDeleteEntries([entry]);
      },
    });
  });

  return {
    drawerName,
    drawerDetail,
    drawerEditing,
    setDrawerEditing,
    drawerDraft,
    setDrawerDraft,
    drawerLoading,
    drawerReadOnly,
    closeDrawer,
    resetDrawer,
    openAsset,
    saveAsset,
    reloadDrawerIfOpen,
    runDeleteEntries,
    requestDeleteAsset,
    deleteAsset: requestDeleteAsset,
  };
}
