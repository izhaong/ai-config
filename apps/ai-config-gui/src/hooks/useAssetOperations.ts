import { useMemoizedFn } from "ahooks";
import { useTranslation } from "react-i18next";

import {
  deployAsset,
  importAsset,
  retractAsset,
  revealPath,
} from "../api/tauriAssets";
import { PLATFORM_NAME } from "../platformIcons";
import type { ConfirmRequest } from "./useConfirm";
import type { ToastKind } from "./useToast";
import type { useAssetBrowser } from "./useAssetBrowser";
import type { useAssetDrawer } from "./useAssetDrawer";
import type {
  AssetKind,
  DeployPlatform,
  Platform,
  PlatformAssetEntry,
} from "../types";
import { canRetract, hasSourceEntry, isSourcePlatform } from "../types";
import {
  resolveBatchEntryPlatformAction,
  resolveBatchPlatformToggleMode,
  resolveEntryPlatformToggleAction,
  type EntryPlatformToggleAction,
} from "../utils/entryPlatformToggle";

type Browser = ReturnType<typeof useAssetBrowser>;
type Drawer = ReturnType<typeof useAssetDrawer>;

interface UseAssetOperationsOptions {
  browser: Browser;
  drawer: Drawer;
  showToast: (kind: ToastKind, text: string) => void;
  requestConfirm: (req: ConfirmRequest) => void;
  dismissConfirm: () => void;
  setBusy: (busy: boolean) => void;
}

export function useAssetOperations({
  browser,
  drawer,
  showToast,
  requestConfirm,
  dismissConfirm,
  setBusy,
}: UseAssetOperationsOptions) {
  const { t } = useTranslation();

  const {
    activeProject,
    activePlatform,
    activeKind,
    browsingSource,
    selectedEntries,
    issueMap,
    browsePath,
    refreshView,
    clearSelection,
  } = browser;

  const { runDeleteEntries } = drawer;
  const kindLabel = t(`assetKind.${activeKind}`);
  const irreversibleHint = t("confirm.irreversible");

  const toggleContext = useMemoizedFn(() => ({
    activePlatform,
    browsingSource,
    issueKey: (plat: DeployPlatform, kind: AssetKind) =>
      issueMap.has(`${plat}:${kind}`),
  }));

  const runPlatformAction = useMemoizedFn(
    async (
      entry: PlatformAssetEntry,
      plat: Platform,
      action: EntryPlatformToggleAction,
    ): Promise<{ action: EntryPlatformToggleAction; message?: string }> => {
      if (action === "skip" || action === "unsupported") {
        return { action };
      }

      // 删源只能经删除按钮（源视图 + 确认），禁止从平台 icon / 批量同步误入
      if (action === "delete_source") {
        return { action: "skip" };
      }

      if (action === "import") {
        if (isSourcePlatform(activePlatform)) {
          return { action: "skip" };
        }
        const message = await importAsset(
          entry.kind,
          entry.name,
          activeProject,
          activePlatform,
        );
        return { action, message };
      }

      const deployPlat = plat as DeployPlatform;
      if (action === "retract") {
        const message = await retractAsset(
          entry.kind,
          entry.name,
          activeProject,
          deployPlat,
        );
        return { action, message };
      }

      if (
        !browsingSource &&
        !isSourcePlatform(activePlatform) &&
        activePlatform === deployPlat &&
        !hasSourceEntry(entry)
      ) {
        await importAsset(
          entry.kind,
          entry.name,
          activeProject,
          activePlatform,
        );
      }

      const message = await deployAsset(
        entry.kind,
        entry.name,
        activeProject,
        deployPlat,
      );
      return { action: "deploy", message };
    },
  );

  const showToggleSuccessToast = useMemoizedFn(
    (action: EntryPlatformToggleAction, message?: string) => {
      if (action === "import") {
        showToast("ok", t("toast.importSuccess", { message: message ?? "" }));
        return;
      }
      if (message) {
        showToast("ok", message);
        return;
      }
      if (action === "delete_source") {
        showToast("ok", t("toast.deletedCount", { count: 1 }));
      }
    },
  );

  const showToggleErrorToast = useMemoizedFn(
    (action: EntryPlatformToggleAction, error: unknown) => {
      switch (action) {
        case "delete_source":
          showToast("err", t("toast.deleteFailed", { errors: String(error) }));
          break;
        case "import":
          showToast("err", t("toast.importFailed", { error: error }));
          break;
        case "retract":
          showToast("err", t("toast.retractFailed", { error: error }));
          break;
        case "deploy":
          showToast("err", t("toast.deployFailed", { error: error }));
          break;
        default:
          showToast("err", String(error));
      }
    },
  );

  const handlePlatformToggle = useMemoizedFn(
    (entry: PlatformAssetEntry, plat: Platform) => {
      const planned = resolveEntryPlatformToggleAction(
        entry,
        plat,
        toggleContext(),
      );
      if (planned === "skip" || planned === "unsupported") {
        return;
      }

      void (async () => {
        setBusy(true);
        try {
          const result = await runPlatformAction(entry, plat, planned);
          if (result.action === "skip" || result.action === "unsupported") {
            return;
          }
          await refreshView(false);
          showToggleSuccessToast(result.action, result.message);
        } catch (e) {
          showToggleErrorToast(planned, e);
        } finally {
          setBusy(false);
        }
      })();
    },
  );

  const reportBatchToggleResults = useMemoizedFn(
    (counts: Record<EntryPlatformToggleAction | "failed", number>) => {
      const ok =
        counts.deploy + counts.retract + counts.import + counts.delete_source;
      const failed = counts.failed;

      if (ok === 0 && failed === 0) {
        showToast("err", t("toast.batchNoOps"));
        return;
      }

      if (failed > 0) {
        if (ok === 0) {
          showToast("err", t("toast.batchToggleAllFailed", { count: failed }));
          return;
        }
        showToast(
          "err",
          t("toast.batchTogglePartial", {
            ok,
            failed,
            deploy: counts.deploy,
            retract: counts.retract,
            import: counts.import,
            deleted: counts.delete_source,
          }),
        );
        return;
      }

      const kinds = [
        counts.deploy > 0,
        counts.retract > 0,
        counts.import > 0,
        counts.delete_source > 0,
      ].filter(Boolean).length;

      if (kinds === 1) {
        if (counts.retract > 0) {
          showToast("ok", t("toast.batchRetracted", { count: counts.retract }));
        } else if (counts.deploy > 0) {
          showToast("ok", t("toast.batchDeployed", { count: counts.deploy }));
        } else if (counts.import > 0) {
          showToast("ok", t("toast.batchImported", { count: counts.import }));
        } else if (counts.delete_source > 0) {
          showToast(
            "ok",
            t("toast.deletedCount", { count: counts.delete_source }),
          );
        }
        return;
      }

      showToast(
        "ok",
        t("toast.batchToggled", {
          deploy: counts.deploy,
          retract: counts.retract,
          import: counts.import,
          deleted: counts.delete_source,
        }),
      );
    },
  );

  const batchSyncToPlatform = useMemoizedFn(async (plat: Platform) => {
    if (selectedEntries.length === 0) {
      showToast("err", t("toast.selectRowsFirst"));
      return;
    }

    if (plat === "aiconfig" && isSourcePlatform(activePlatform)) {
      return;
    }

    const batchMode = resolveBatchPlatformToggleMode(selectedEntries, plat);

    // ai-config 批量 icon 仅支持「导入到源」；删源请用删除按钮
    if (plat === "aiconfig" && batchMode === "retract_all") {
      showToast("err", t("toast.batchAiconfigDeleteBlocked"));
      return;
    }

    const deployPlat = plat !== "aiconfig" ? (plat as DeployPlatform) : null;
    if (
      deployPlat &&
      issueMap.has(`${deployPlat}:${activeKind}`) &&
      selectedEntries.every((entry) =>
        issueMap.has(`${deployPlat}:${entry.kind}`),
      )
    ) {
      return;
    }

    setBusy(true);
    const counts: Record<EntryPlatformToggleAction | "failed", number> = {
      deploy: 0,
      retract: 0,
      import: 0,
      delete_source: 0,
      skip: 0,
      unsupported: 0,
      failed: 0,
    };

    try {
      for (const entry of selectedEntries) {
        const planned = resolveBatchEntryPlatformAction(
          entry,
          plat,
          batchMode,
          toggleContext(),
        );
        if (planned === "skip" || planned === "unsupported" || planned === "delete_source") {
          counts.skip += 1;
          continue;
        }
        try {
          const result = await runPlatformAction(entry, plat, planned);
          if (result.action === "skip" || result.action === "unsupported") {
            counts[result.action] += 1;
          } else {
            counts[result.action] += 1;
          }
        } catch {
          counts.failed += 1;
        }
      }
      await refreshView(false);
      reportBatchToggleResults(counts);
    } finally {
      setBusy(false);
    }
  });

  const runBatchDelete = useMemoizedFn(async () => {
    setBusy(true);
    try {
      if (browsingSource) {
        await runDeleteEntries(selectedEntries);
      } else if (!isSourcePlatform(activePlatform)) {
        const plat = activePlatform as DeployPlatform;
        let ok = 0;
        for (const entry of selectedEntries) {
          try {
            if (canRetract(entry.states[plat])) {
              await retractAsset(entry.kind, entry.name, activeProject, plat);
              ok += 1;
            }
          } catch (e) {
            showToast("err", t("toast.deleteFailed", { errors: String(e) }));
          }
        }
        clearSelection();
        await refreshView(false);
        if (ok > 0) {
          showToast("ok", t("toast.batchRetracted", { count: ok }));
        } else {
          showToast("err", t("toast.batchNoOps"));
        }
        dismissConfirm();
      }
    } finally {
      setBusy(false);
    }
  });

  const requestBatchDelete = useMemoizedFn(() => {
    if (selectedEntries.length === 0) {
      showToast("err", t("toast.selectRowsFirst"));
      return;
    }
    requestConfirm({
      title: browsingSource
        ? t("confirm.batchDelete")
        : t("confirm.batchRetract"),
      message: browsingSource
        ? t("confirm.batchDeleteMessage", {
            count: selectedEntries.length,
            label: kindLabel,
            irreversible: irreversibleHint,
          })
        : t("confirm.batchRetractPlatformMessage", {
            count: selectedEntries.length,
            platform: PLATFORM_NAME[activePlatform],
          }),
      confirmLabel: browsingSource ? t("confirm.delete") : t("confirm.retract"),
      onConfirm: () => runBatchDelete(),
    });
  });

  const runDeleteEntry = useMemoizedFn(async (entry: PlatformAssetEntry) => {
    setBusy(true);
    try {
      if (browsingSource) {
        await runDeleteEntries([entry]);
        dismissConfirm();
      } else if (!isSourcePlatform(activePlatform)) {
        const plat = activePlatform as DeployPlatform;
        try {
          if (canRetract(entry.states[plat])) {
            await retractAsset(entry.kind, entry.name, activeProject, plat);
            await refreshView(false);
            showToast("ok", t("toast.retractedCount", { count: 1 }));
          } else {
            showToast("err", t("toast.batchNoOps"));
          }
        } catch (e) {
          showToast("err", t("toast.retractFailed", { error: e }));
        } finally {
          dismissConfirm();
        }
      }
    } finally {
      setBusy(false);
    }
  });

  const requestDeleteEntry = useMemoizedFn((entry: PlatformAssetEntry) => {
    requestConfirm({
      title: browsingSource
        ? t("confirm.deleteSource")
        : t("confirm.retractEntry"),
      message: browsingSource
        ? t("confirm.deleteSourceMessage", {
            label: kindLabel,
            name: entry.name,
            irreversible: irreversibleHint,
          })
        : t("confirm.retractEntryPlatformMessage", {
            name: entry.name,
            platform: PLATFORM_NAME[activePlatform],
          }),
      confirmLabel: browsingSource ? t("confirm.delete") : t("confirm.retract"),
      onConfirm: () => runDeleteEntry(entry),
    });
  });

  const openBrowseFolder = useMemoizedFn(async () => {
    if (!browsePath) {
      showToast("err", t("toast.noBrowsePath"));
      return;
    }
    try {
      await revealPath(browsePath);
    } catch (e) {
      showToast("err", t("toast.openFolderFailed", { error: e }));
    }
  });

  return {
    handlePlatformToggle,
    batchSyncToPlatform,
    requestBatchDelete,
    requestDeleteEntry,
    openBrowseFolder,
  };
}
