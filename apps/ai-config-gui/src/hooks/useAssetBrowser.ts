import { useLatest, useMemoizedFn, useRequest, useUnmount } from "ahooks";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { fetchPlatformKindPaths, fetchPlatformList } from "../api/tauriAssets";
import { entryKey } from "../utils/entryKey";
import { canOpenEntry as canOpenAssetEntry } from "../utils/canOpenEntry";
import type { ToastKind } from "./useToast";
import type {
  AssetKind,
  DeployPlatform,
  DoctorSummary,
  Platform,
  PlatformAssetEntry,
  PlatformAssetList,
  PlatformKindPath,
} from "../types";
import { isSourcePlatform } from "../types";

interface UseAssetBrowserOptions {
  showToast: (kind: ToastKind, text: string) => void;
  doctor: DoctorSummary | null;
  activeProject: string;
  setActiveProject: (name: string) => void;
  onContextReset?: () => void;
  reloadDrawerIfOpen?: (list: PlatformAssetList | null) => Promise<void>;
}

export function useAssetBrowser({
  showToast,
  doctor,
  activeProject,
  setActiveProject,
  onContextReset,
  reloadDrawerIfOpen,
}: UseAssetBrowserOptions) {
  const { t } = useTranslation();
  const [activePlatform, setActivePlatform] = useState<Platform>("cursor");
  const [activeKind, setActiveKind] = useState<AssetKind>("skill");
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [manualLoading, setManualLoading] = useState(false);

  const reloadDrawerLatest = useLatest(reloadDrawerIfOpen);
  const onContextResetLatest = useLatest(onContextReset);
  const showToastLatest = useLatest(showToast);
  const tLatest = useLatest(t);

  const browsingSource = isSourcePlatform(activePlatform);

  const issues = doctor?.platform_capability_issues ?? [];
  const issueMap = useMemo(
    () => new Map(issues.map((i) => [`${i.platform}:${i.kind}`, i.reason])),
    [issues],
  );

  const issueReasonFor = useMemoizedFn(
    (entry: PlatformAssetEntry, plat: DeployPlatform) =>
      issueMap.get(`${plat}:${entry.kind}`),
  );

  const [platformKindPaths, setPlatformKindPaths] = useState<
    PlatformKindPath[]
  >([]);

  useRequest(() => fetchPlatformKindPaths(activeProject, activeKind), {
    refreshDeps: [activeProject, activeKind],
    onSuccess: (paths) => setPlatformKindPaths(paths),
    onError: () => setPlatformKindPaths([]),
  });

  const {
    data: platformList,
    loading: listLoading,
    refreshAsync: refreshListAsync,
  } = useRequest(
    () => fetchPlatformList(activeProject, activePlatform, activeKind),
    {
      refreshDeps: [activeProject, activePlatform, activeKind],
      onError: (e) => {
        showToastLatest.current(
          "err",
          tLatest.current("toast.platformListFailed", { error: e }),
        );
      },
    },
  );

  const loading = listLoading || manualLoading;

  const refresh = useMemoizedFn(async () => {
    try {
      return await refreshListAsync();
    } catch (e) {
      showToastLatest.current(
        "err",
        tLatest.current("toast.refreshFailed", { error: e }),
      );
      return null;
    }
  });

  const refreshView = useMemoizedFn(async (showLoading = false) => {
    if (showLoading) setManualLoading(true);
    try {
      const r = await refresh();
      await reloadDrawerLatest.current?.(r ?? null);
    } finally {
      if (showLoading) setManualLoading(false);
    }
  });

  const refreshViewLatest = useLatest(refreshView);
  const unlistenRef = useRef<(() => void) | undefined>(undefined);

  useEffect(() => {
    let disposed = false;

    void listen("assets-changed", () => {
      void refreshViewLatest.current(false);
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlistenRef.current = fn;
      }
    });

    return () => {
      disposed = true;
      unlistenRef.current?.();
      unlistenRef.current = undefined;
    };
  }, [refreshViewLatest]);

  useUnmount(() => {
    unlistenRef.current?.();
  });

  useEffect(() => {
    setSelectedKeys(new Set());
    onContextResetLatest.current?.();
  }, [activeKind, activeProject, activePlatform, onContextResetLatest]);

  const visible = useMemo(() => platformList?.entries ?? [], [platformList]);

  const visibleKeys = useMemo(() => new Set(visible.map(entryKey)), [visible]);

  const selectedEntries = useMemo(
    () => visible.filter((e) => selectedKeys.has(entryKey(e))),
    [visible, selectedKeys],
  );

  const allVisibleSelected =
    visible.length > 0 && visible.every((e) => selectedKeys.has(entryKey(e)));

  const browsePath = useMemo(() => {
    const row = platformKindPaths.find((p) => p.platform === activePlatform);
    return row?.path ?? "";
  }, [platformKindPaths, activePlatform]);

  const platformPathMap = useMemo(
    () => new Map(platformKindPaths.map((r) => [r.platform, r])),
    [platformKindPaths],
  );

  const platformKindUnsupported =
    !isSourcePlatform(activePlatform) &&
    !!issueMap.get(`${activePlatform}:${activeKind}`);

  const canOpenEntry = useMemoizedFn((entry: PlatformAssetEntry) =>
    canOpenAssetEntry(entry, activePlatform),
  );

  const togglePlatformBrowse = useMemoizedFn((plat: Platform) => {
    setActivePlatform(plat);
  });

  const toggleSelect = useMemoizedFn((key: string, checked: boolean) => {
    setSelectedKeys((prev) => {
      const next = new Set(prev);
      if (checked) next.add(key);
      else next.delete(key);
      return next;
    });
  });

  const toggleSelectAll = useMemoizedFn(() => {
    if (allVisibleSelected) {
      setSelectedKeys(new Set());
    } else {
      setSelectedKeys(new Set(visible.map(entryKey)));
    }
  });

  useEffect(() => {
    setSelectedKeys((prev) => {
      const next = new Set([...prev].filter((k) => visibleKeys.has(k)));
      return next.size === prev.size ? prev : next;
    });
  }, [visibleKeys]);

  const clearSelection = useMemoizedFn(() => setSelectedKeys(new Set()));

  return {
    activeProject,
    setActiveProject,
    activePlatform,
    setActivePlatform,
    activeKind,
    setActiveKind,
    platformList: platformList ?? null,
    platformKindPaths,
    loading,
    selectedKeys,
    setSelectedKeys,
    visible,
    visibleKeys,
    selectedEntries,
    allVisibleSelected,
    browsePath,
    issueMap,
    issueReasonFor,
    browsingSource,
    platformPathMap,
    platformKindUnsupported,
    canOpenEntry,
    togglePlatformBrowse,
    toggleSelect,
    toggleSelectAll,
    clearSelection,
    refresh,
    refreshView,
  };
}
