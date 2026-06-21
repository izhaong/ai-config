import { useMemoizedFn } from "ahooks";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  addSkillFromRemote,
  deployAsset,
  deployAssetFromPlatform,
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
import { canRemoveFromPlatform, hasSourceEntry, isSourcePlatform } from "../types";
import {
  resolveBatchEntryPlatformAction,
  resolveBatchPlatformToggleMode,
  resolveEntryPlatformToggleAction,
  type EntryPlatformToggleAction,
} from "../utils/entryPlatformToggle";
import { canUpdateEntry, deployPlatformsForUpdate } from "../utils/entryUpdate";

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

      // 删源 + 全平台收回：仅删除按钮 + 确认框
      if (action === "delete_source") {
        return { action: "skip" };
      }

      if (plat === "aiconfig" && isSourcePlatform(activePlatform)) {
        if (action === "import") {
          return { action: "skip" };
        }
      }

      // MCP：每条 server 独立条目，retract 不会动源 / 其它 server
      if (entry.kind === "mcp" && action === "retract") {
        const message = await retractAsset(
          entry.kind,
          entry.name,
          activeProject,
          plat,
        );
        return { action, message };
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

      if (action === "platform_copy") {
        const message = await deployAssetFromPlatform(
          entry.kind,
          entry.name,
          activeProject,
          activePlatform as DeployPlatform,
          plat as DeployPlatform,
        );
        return { action: "deploy", message };
      }

      const deployPlat = plat as DeployPlatform;
      if (action === "retract") {
        const message = await retractAsset(
          entry.kind,
          entry.name,
          activeProject,
          plat,
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

  const deployEntryToPlatforms = useMemoizedFn(
    async (
      entry: PlatformAssetEntry,
      platforms: DeployPlatform[],
    ): Promise<{ ok: number; failed: number }> => {
      let ok = 0;
      let failed = 0;
      const ctx = toggleContext();
      for (const plat of platforms) {
        if (ctx.issueKey(plat, entry.kind)) {
          continue;
        }
        try {
          await deployAsset(entry.kind, entry.name, activeProject, plat);
          ok += 1;
        } catch {
          failed += 1;
        }
      }
      return { ok, failed };
    },
  );

  const handleUpdateEntry = useMemoizedFn((entry: PlatformAssetEntry) => {
    const platforms = deployPlatformsForUpdate(entry).filter(
      (plat) => !toggleContext().issueKey(plat, entry.kind),
    );
    if (platforms.length === 0) {
      showToast("err", t("toast.updateNothing"));
      return;
    }

    void (async () => {
      setBusy(true);
      try {
        const { ok, failed } = await deployEntryToPlatforms(entry, platforms);
        await refreshView(false);
        if (ok === 0 && failed > 0) {
          showToast("err", t("toast.updateFailed", { count: failed }));
        } else if (failed > 0) {
          showToast("err", t("toast.updatePartial", { ok, failed }));
        } else {
          showToast("ok", t("toast.updatedCount", { count: ok }));
        }
      } finally {
        setBusy(false);
      }
    })();
  });

  const batchUpdateEntries = useMemoizedFn(() => {
    if (selectedEntries.length === 0) {
      showToast("err", t("toast.selectRowsFirst"));
      return;
    }

    const ctx = toggleContext();
    const targets = selectedEntries.filter((entry) =>
      canUpdateEntry(entry, ctx.issueKey),
    );
    if (targets.length === 0) {
      showToast("err", t("toast.updateNothing"));
      return;
    }

    void (async () => {
      setBusy(true);
      let ok = 0;
      let failed = 0;
      try {
        for (const entry of targets) {
          const platforms = deployPlatformsForUpdate(entry).filter(
            (plat) => !ctx.issueKey(plat, entry.kind),
          );
          const result = await deployEntryToPlatforms(entry, platforms);
          ok += result.ok;
          failed += result.failed;
        }
        await refreshView(false);
        if (ok === 0 && failed > 0) {
          showToast("err", t("toast.updateFailed", { count: failed }));
        } else if (failed > 0) {
          showToast("err", t("toast.updatePartial", { ok, failed }));
        } else {
          showToast("ok", t("toast.updatedCount", { count: ok }));
        }
      } finally {
        setBusy(false);
      }
    })();
  });

  const reportBatchToggleResults = useMemoizedFn(
    (counts: Record<EntryPlatformToggleAction | "failed", number>) => {
      const ok =
        counts.deploy +
        counts.retract +
        counts.import +
        counts.platform_copy +
        counts.delete_source;
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
        counts.platform_copy > 0,
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

    const batchMode = resolveBatchPlatformToggleMode(
      selectedEntries,
      plat,
      activePlatform,
      browsingSource,
    );

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
      platform_copy: 0,
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
        if (
          planned === "skip" ||
          planned === "unsupported" ||
          planned === "delete_source"
        ) {
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
            if (canRemoveFromPlatform(entry.states[plat])) {
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
          if (canRemoveFromPlatform(entry.states[plat])) {
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

  const [addSkillOpen, setAddSkillOpen] = useState(false);
  const [marketplaceOpen, setMarketplaceOpen] = useState(false);

  const openAddSkill = useMemoizedFn(() => {
    if (activeKind !== "skill") return;
    setAddSkillOpen(true);
  });

  const closeAddSkill = useMemoizedFn(() => {
    setAddSkillOpen(false);
  });

  const submitAddSkill = useMemoizedFn(
    async (source: string, skillName?: string) => {
      setBusy(true);
      try {
        const msg = await addSkillFromRemote(
          source,
          skillName,
          activeProject,
          activePlatform,
        );
        showToast("ok", msg);
        setAddSkillOpen(false);
        await refreshView(true);
      } catch (e) {
        showToast("err", t("toast.addSkillFailed", { error: e }));
      } finally {
        setBusy(false);
      }
    },
  );

  const openAddMarketplace = useMemoizedFn(() => {
    if (activeKind !== "skill") return;
    setMarketplaceOpen(true);
  });

  const closeAddMarketplace = useMemoizedFn(() => {
    setMarketplaceOpen(false);
  });

  const handleMarketplaceImported = useMemoizedFn(async (message: string) => {
    showToast("ok", message);
    setMarketplaceOpen(false);
    await refreshView(true);
  });

  const handleMarketplaceError = useMemoizedFn((message: string) => {
    showToast("err", message);
  });

  return {
    handlePlatformToggle,
    handleUpdateEntry,
    batchUpdateEntries,
    batchSyncToPlatform,
    requestBatchDelete,
    requestDeleteEntry,
    openBrowseFolder,
    addSkillOpen,
    openAddSkill,
    closeAddSkill,
    submitAddSkill,
    marketplaceOpen,
    openAddMarketplace,
    closeAddMarketplace,
    handleMarketplaceImported,
    handleMarketplaceError,
  };
}
