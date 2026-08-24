import { useMemoizedFn } from "ahooks";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  addSkillFromRemote,
  applyImportToSourcePlan,
  applyProjectionPlan,
  fetchImportToSourcePlan,
  fetchProjectionPlan,
  revealPath,
  retractProjectionPlan,
  saveAsset,
  transferAssets,
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
  ImportPlan,
  ProjectionReview,
  ProjectItem,
} from "../types";
import { hasSourceEntry, isSourcePlatform } from "../types";
import {
  resolveBatchPlatformToggleMode,
  resolveEntryPlatformToggleAction,
  type EntryPlatformToggleAction,
} from "../utils/entryPlatformToggle";
import { canUpdateEntry, deployPlatformsForUpdate } from "../utils/entryUpdate";

type Browser = ReturnType<typeof useAssetBrowser>;
type Drawer = ReturnType<typeof useAssetDrawer>;

interface PendingProjectionReview {
  review: ProjectionReview;
  retract: boolean;
}

interface PendingImportReview {
  entry: PlatformAssetEntry;
  sourcePlatform: DeployPlatform;
  plan: ImportPlan;
}

interface UseAssetOperationsOptions {
  browser: Browser;
  drawer: Drawer;
  projects: ProjectItem[];
  showToast: (kind: ToastKind, text: string) => void;
  requestConfirm: (req: ConfirmRequest) => void;
  dismissConfirm: () => void;
  setBusy: (busy: boolean) => void;
}

export function useAssetOperations({
  browser,
  drawer,
  projects,
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
  const [projectionReview, setProjectionReview] =
    useState<PendingProjectionReview | null>(null);
  const [importReview, setImportReview] = useState<PendingImportReview | null>(null);

  const toggleContext = useMemoizedFn(() => ({
    activePlatform,
    browsingSource,
    issueKey: (plat: DeployPlatform, kind: AssetKind) =>
      issueMap.has(`${plat}:${kind}`),
  }));

  const openProjectionReview = useMemoizedFn(async (retract: boolean) => {
    setBusy(true);
    try {
      const review = await fetchProjectionPlan(activeProject, retract);
      setProjectionReview({ review, retract });
    } catch (e) {
      showToast("err", t("projection.reviewFailed", { error: e }));
    } finally {
      setBusy(false);
    }
  });

  const dismissProjectionReview = useMemoizedFn(() => {
    setProjectionReview(null);
  });

  const openImportReview = useMemoizedFn(
    async (entry: PlatformAssetEntry, sourcePlatform: DeployPlatform) => {
      setBusy(true);
      try {
        const plan = await fetchImportToSourcePlan(
          entry.kind,
          entry.name,
          activeProject,
          sourcePlatform,
        );
        setImportReview({ entry, sourcePlatform, plan });
      } catch (e) {
        showToast("err", t("projection.importFailed", { error: e }));
      } finally {
        setBusy(false);
      }
    },
  );

  const dismissImportReview = useMemoizedFn(() => setImportReview(null));

  const confirmImportReview = useMemoizedFn(async () => {
    if (!importReview) return;
    const action = importReview.plan.actions[0];
    if (!action) return;
    setBusy(true);
    try {
      const report = await applyImportToSourcePlan(
        importReview.entry.kind,
        importReview.entry.name,
        activeProject,
        importReview.sourcePlatform,
        importReview.plan.plan_digest,
        [action.action_id],
      );
      setImportReview(null);
      await refreshView(false);
      showToast("ok", t("projection.imported", { count: report.applied }));
    } catch (e) {
      showToast("err", t("projection.importFailed", { error: e }));
    } finally {
      setBusy(false);
    }
  });

  const confirmProjectionReview = useMemoizedFn(async () => {
    if (!projectionReview) return;
    setBusy(true);
    try {
      const apply = projectionReview.retract
        ? await retractProjectionPlan(
            activeProject,
            projectionReview.review.plan_digest,
          )
        : await applyProjectionPlan(
            activeProject,
            projectionReview.review.plan_digest,
          );
      setProjectionReview(null);
      await refreshView(false);
      showToast(
        "ok",
        t("projection.applied", { changed: apply.changed, unchanged: apply.unchanged }),
      );
    } catch (e) {
      showToast("err", t("projection.applyFailed", { error: e }));
    } finally {
      setBusy(false);
    }
  });

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

      if (plat === "agentmanager" && isSourcePlatform(activePlatform)) {
        if (action === "import") {
          return { action: "skip" };
        }
      }

      if (action === "import") {
        if (isSourcePlatform(activePlatform)) {
          return { action: "skip" };
        }
        await openImportReview(entry, activePlatform as DeployPlatform);
        return { action: "skip" };
      }

      // Projection writes are scope-level and digest-bound. Callers open the reviewed modal
      // before reaching this per-entry import-only fallback.
      return { action: "skip" };
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

  const executePlatformToggle = useMemoizedFn(
    async (
      entry: PlatformAssetEntry,
      plat: Platform,
      planned: EntryPlatformToggleAction,
    ) => {
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
        if (
          planned === "skip" &&
          plat === "agentmanager" &&
          hasSourceEntry(entry)
        ) {
          showToast("ok", t("platformView.importManaged"));
        }
        return;
      }
      if (planned === "deploy" || planned === "retract") {
        void openProjectionReview(planned === "retract");
        return;
      }
      void executePlatformToggle(entry, plat, planned);
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

    void openProjectionReview(false);
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

    void openProjectionReview(false);
  });

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

    const deployPlat = plat !== "agentmanager" ? (plat as DeployPlatform) : null;
    if (
      deployPlat &&
      issueMap.has(`${deployPlat}:${activeKind}`) &&
      selectedEntries.every((entry) =>
        issueMap.has(`${deployPlat}:${entry.kind}`),
      )
    ) {
      return;
    }

    void openProjectionReview(batchMode === "retract_all");
  });

  const runBatchDelete = useMemoizedFn(async () => {
    setBusy(true);
    try {
      if (browsingSource) {
        await runDeleteEntries(selectedEntries);
      } else if (!isSourcePlatform(activePlatform)) {
        dismissConfirm();
        void openProjectionReview(true);
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
        dismissConfirm();
        void openProjectionReview(true);
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
  const [addMcpOpen, setAddMcpOpen] = useState(false);
  const [marketplaceOpen, setMarketplaceOpen] = useState(false);
  const [copyToProjectOpen, setCopyToProjectOpen] = useState(false);

  const openCopyToProject = useMemoizedFn(() => {
    if (selectedEntries.length === 0) {
      showToast("err", t("toast.selectRowsFirst"));
      return;
    }
    const hasCopyTarget =
      activeProject !== "user-global" || projects.length > 0;
    if (!hasCopyTarget) {
      showToast("err", t("toast.copyToProjectNoTargets"));
      return;
    }
    setCopyToProjectOpen(true);
  });

  const closeCopyToProject = useMemoizedFn(() => {
    setCopyToProjectOpen(false);
  });

  const submitCopyToProject = useMemoizedFn(async (toProject: string) => {
    if (toProject === activeProject) {
      showToast("err", t("toast.copyToProjectSameTarget"));
      return;
    }
    setBusy(true);
    try {
      // 非源视图只允许先经 digest-bound 导入审阅写回 source，不能在复制操作中绕过审阅。
      if (!browsingSource && !isSourcePlatform(activePlatform)) {
        if (selectedEntries.some((entry) => !hasSourceEntry(entry))) {
          showToast("err", t("projection.importRequiredBeforeCopy"));
          return;
        }
      }

      const items = selectedEntries.map((entry) => ({
        kind: entry.kind,
        name: entry.name,
      }));
      const msg = await transferAssets(activeProject, toProject, items);
      showToast("ok", msg);
      setCopyToProjectOpen(false);
      clearSelection();
      await refreshView(false);
    } catch (e) {
      showToast("err", t("toast.copyToProjectFailed", { error: e }));
    } finally {
      setBusy(false);
    }
  });

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

  const openAddMcp = useMemoizedFn(() => {
    if (activeKind !== "mcp" || !browsingSource) return;
    setAddMcpOpen(true);
  });

  const closeAddMcp = useMemoizedFn(() => {
    setAddMcpOpen(false);
  });

  const submitAddMcp = useMemoizedFn(
    async (name: string, configJson: string) => {
      setBusy(true);
      try {
        const msg = await saveAsset("mcp", name, configJson, activeProject);
        showToast("ok", msg);
        setAddMcpOpen(false);
        await refreshView(true);
      } catch (e) {
        showToast("err", t("toast.addMcpFailed", { error: e }));
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

  const handleHookLifecyclePairToggle = useMemoizedFn(
    async (
      entry: PlatformAssetEntry,
      plan: Array<{ lifecycle: string; enabled: boolean }>,
    ) => {
      if (entry.kind !== "hook" || plan.length === 0) return;
      void entry;
      void plan;
      // Hook lifecycle belongs to the same source-owned projection contract;
      // the review lists every affected hook before a write can occur.
      await openProjectionReview(false);
    },
  );

  return {
    handlePlatformToggle,
    handleHookLifecyclePairToggle,
    projectionReview,
    dismissProjectionReview,
    confirmProjectionReview,
    importReview,
    dismissImportReview,
    confirmImportReview,
    handleUpdateEntry,
    batchUpdateEntries,
    batchSyncToPlatform,
    requestBatchDelete,
    requestDeleteEntry,
    openBrowseFolder,
    copyToProjectOpen,
    openCopyToProject,
    closeCopyToProject,
    submitCopyToProject,
    addSkillOpen,
    openAddSkill,
    closeAddSkill,
    submitAddSkill,
    addMcpOpen,
    openAddMcp,
    closeAddMcp,
    submitAddMcp,
    marketplaceOpen,
    openAddMarketplace,
    closeAddMarketplace,
    handleMarketplaceImported,
    handleMarketplaceError,
  };
}
