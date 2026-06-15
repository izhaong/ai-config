/**
 * ai-config GUI 主组件 — 统一列表 + 批量操作 + 资产编辑抽屉
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { AssetDrawer } from "./AssetDrawer";
import { AssetRow } from "./AssetRow";
import { ConfirmModal } from "./ConfirmModal";
import { RegisterProjectModal } from "./RegisterProjectModal";
import { LanguageSwitcher } from "./LanguageSwitcher";
import { assetKindLabel } from "./i18n/labels";
import {
  ALL_PLATFORMS,
  ASSET_KINDS,
  canDeploy,
  canRetract,
  DEPLOY_PLATFORMS,
  hasSourceEntry,
  isSourcePlatform,
  type AssetDetail,
  type AssetKind,
  type DeployPlatform,
  type DoctorSummary,
  type Platform,
  type PlatformAssetEntry,
  type PlatformAssetList,
  type PlatformKindPath,
  type ProjectItem,
} from "./types";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "./platformIcons";

function entryKey(entry: { kind: AssetKind; name: string }): string {
  return `${entry.kind}:${entry.name}`;
}

export function App() {
  const { t } = useTranslation();
  const [activeProject, setActiveProject] = useState<string>("user-global");
  const [activePlatform, setActivePlatform] = useState<Platform>("cursor");
  const [activeKind, setActiveKind] = useState<AssetKind>("skill");
  const [projects, setProjects] = useState<ProjectItem[]>([]);
  const [platformList, setPlatformList] = useState<PlatformAssetList | null>(null);
  const [platformKindPaths, setPlatformKindPaths] = useState<PlatformKindPath[]>([]);
  const [doctor, setDoctor] = useState<DoctorSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ kind: "err" | "ok"; text: string } | null>(null);
  const [confirm, setConfirm] = useState<{
    title: string;
    message: string;
    confirmLabel?: string;
    onConfirm: () => Promise<void>;
  } | null>(null);
  const [registerProjectOpen, setRegisterProjectOpen] = useState(false);
  const [registerProjectBusy, setRegisterProjectBusy] = useState(false);

  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [drawerName, setDrawerName] = useState<string | null>(null);
  const [drawerDetail, setDrawerDetail] = useState<AssetDetail | null>(null);
  const [drawerEditing, setDrawerEditing] = useState(false);
  const [drawerDraft, setDrawerDraft] = useState("");
  const [drawerLoading, setDrawerLoading] = useState(false);

  const reqIdRef = useRef(0);
  const drawerNameRef = useRef<string | null>(null);
  const drawerEditingRef = useRef(false);
  const kindLabel = assetKindLabel(t, activeKind);
  const browsingSource = isSourcePlatform(activePlatform);

  useEffect(() => {
    drawerNameRef.current = drawerName;
  }, [drawerName]);

  useEffect(() => {
    drawerEditingRef.current = drawerEditing;
  }, [drawerEditing]);

  useEffect(() => {
    invoke<DoctorSummary>("cmd_doctor").then(setDoctor).catch(console.error);
    invoke<ProjectItem[]>("cmd_projects_list")
      .then(setProjects)
      .catch((e) =>
        setToast({ kind: "err", text: String(e) })
      );
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount once
  }, []);

  useEffect(() => {
    invoke<Array<{ platform: string; path: string; supported: boolean }>>(
      "cmd_platform_kind_paths",
      { project: activeProject, kind: activeKind }
    )
      .then((rows) =>
        setPlatformKindPaths(
          rows.map((r) => ({
            platform: r.platform as Platform,
            path: r.path,
            supported: r.supported,
          }))
        )
      )
      .catch(() => setPlatformKindPaths([]));
  }, [activeProject, activeKind]);

  useEffect(() => {
    const myReq = ++reqIdRef.current;
    setLoading(true);
    invoke<PlatformAssetList>("cmd_list_platform", {
      project: activeProject,
      platform: activePlatform,
      kind: activeKind,
    })
      .then((r) => {
        if (reqIdRef.current === myReq) setPlatformList(r);
      })
      .catch((e) => {
        if (reqIdRef.current === myReq) {
          setToast({ kind: "err", text: t("toast.platformListFailed", { error: e }) });
          setPlatformList(null);
        }
      })
      .finally(() => {
        if (reqIdRef.current === myReq) setLoading(false);
      });
  }, [activeProject, activePlatform, activeKind, t]);

  const visible = useMemo(
    () => platformList?.entries ?? [],
    [platformList]
  );

  const visibleKeys = useMemo(
    () => new Set(visible.map(entryKey)),
    [visible]
  );

  const selectedEntries = useMemo(
    () => visible.filter((e) => selectedKeys.has(entryKey(e))),
    [visible, selectedKeys]
  );

  const allVisibleSelected =
    visible.length > 0 && visible.every((e) => selectedKeys.has(entryKey(e)));

  const browsePath = useMemo(() => {
    const row = platformKindPaths.find((p) => p.platform === activePlatform);
    return row?.path ?? "";
  }, [platformKindPaths, activePlatform]);

  const issues = doctor?.platform_capability_issues ?? [];
  const issueMap = new Map(issues.map((i) => [`${i.platform}:${i.kind}`, i.reason]));

  const issueReasonFor = useCallback(
    (entry: PlatformAssetEntry, plat: DeployPlatform) =>
      issueMap.get(`${plat}:${entry.kind}`),
    [issueMap]
  );

  const refresh = useCallback(async () => {
    const myReq = ++reqIdRef.current;
    try {
      const r = await invoke<PlatformAssetList>("cmd_list_platform", {
        project: activeProject,
        platform: activePlatform,
        kind: activeKind,
      });
      if (reqIdRef.current === myReq) setPlatformList(r);
      return r;
    } catch (e) {
      if (reqIdRef.current === myReq) {
        setToast({ kind: "err", text: t("toast.refreshFailed", { error: e }) });
      }
      return null;
    }
  }, [activeProject, activePlatform, activeKind, t]);

  const closeDrawer = useCallback(() => {
    setDrawerName(null);
    setDrawerDetail(null);
    setDrawerEditing(false);
    setDrawerDraft("");
  }, []);

  const reloadDrawerIfOpen = useCallback(
    async (listResult: PlatformAssetList | null) => {
      const name = drawerNameRef.current;
      if (!name || !listResult) return;

      const stillExists = listResult.entries.some(
        (e) => e.kind === activeKind && e.name === name && hasSourceEntry(e)
      );
      if (!stillExists) {
        closeDrawer();
        return;
      }

      try {
        const d = await invoke<AssetDetail>(`cmd_${activeKind}_get`, {
          name,
          project: activeProject,
        });
        setDrawerDetail(d);
        if (!drawerEditingRef.current) {
          setDrawerDraft(d.content);
        }
      } catch (e) {
        closeDrawer();
        setToast({
          kind: "err",
          text: t("toast.loadAssetFailed", {
            label: assetKindLabel(t, activeKind),
            error: e,
          }),
        });
      }
    },
    [activeKind, activeProject, closeDrawer, t]
  );

  const refreshView = useCallback(
    async (showLoading = false) => {
      if (showLoading) setLoading(true);
      try {
        const r = await refresh();
        await reloadDrawerIfOpen(r);
      } finally {
        if (showLoading) setLoading(false);
      }
    },
    [refresh, reloadDrawerIfOpen]
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen("assets-changed", () => {
      void refreshView(false);
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshView]);

  useEffect(() => {
    setSelectedKeys(new Set());
    setDrawerName(null);
    setDrawerDetail(null);
    setDrawerEditing(false);
    setDrawerDraft("");
  }, [activeKind, activeProject, activePlatform]);

  const openAsset = useCallback(
    async (name: string) => {
      setDrawerName(name);
      setDrawerEditing(false);
      setDrawerDetail(null);
      setDrawerLoading(true);
      try {
        const d = await invoke<AssetDetail>(`cmd_${activeKind}_get`, {
          name,
          project: activeProject,
        });
        setDrawerDetail(d);
        setDrawerDraft(d.content);
      } catch (e) {
        setToast({
          kind: "err",
          text: t("toast.loadAssetFailed", {
            label: assetKindLabel(t, activeKind),
            error: e,
          }),
        });
        setDrawerName(null);
      } finally {
        setDrawerLoading(false);
      }
    },
    [activeKind, activeProject, t]
  );

  const saveAsset = useCallback(async () => {
    if (!drawerName) return;
    setDrawerLoading(true);
    try {
      const r = await invoke<string>(`cmd_${activeKind}_save`, {
        name: drawerName,
        content: drawerDraft,
        project: activeProject,
      });
      setToast({ kind: "ok", text: r });
      setDrawerEditing(false);
      const d = await invoke<AssetDetail>(`cmd_${activeKind}_get`, {
        name: drawerName,
        project: activeProject,
      });
      setDrawerDetail(d);
      setDrawerDraft(d.content);
      await refresh();
    } catch (e) {
      setToast({ kind: "err", text: t("toast.saveFailed", { error: e }) });
    } finally {
      setDrawerLoading(false);
    }
  }, [drawerName, drawerDraft, activeKind, activeProject, refresh, t]);

  const runDeleteEntries = useCallback(
    async (entries: PlatformAssetEntry[]) => {
      setBusy(true);
      let ok = 0;
      const failed: string[] = [];
      try {
        for (const entry of entries) {
          try {
            await invoke<string>(`cmd_${entry.kind}_delete`, {
              name: entry.name,
              project: activeProject,
            });
            ok += 1;
          } catch (e) {
            failed.push(`${entry.name}: ${e}`);
          }
        }
        setSelectedKeys(new Set());
        if (drawerName && entries.some((e) => e.name === drawerName)) {
          closeDrawer();
        }
        await refresh();
        if (failed.length === 0) {
          setToast({ kind: "ok", text: t("toast.deletedCount", { count: ok }) });
        } else if (ok === 0) {
          setToast({
            kind: "err",
            text: t("toast.deleteFailed", { errors: failed.join("；") }),
          });
        } else {
          setToast({
            kind: "err",
            text: t("toast.deletePartial", {
              ok,
              failed: failed.length,
              errors: failed.join("；"),
            }),
          });
        }
      } finally {
        setBusy(false);
        setConfirm(null);
      }
    },
    [activeProject, drawerName, closeDrawer, refresh, t]
  );

  const requestDeleteAsset = useCallback(() => {
    if (!drawerName || !browsingSource) return;
    const entry = visible.find((e) => e.name === drawerName);
    if (!entry) {
      setToast({ kind: "err", text: t("toast.entryNotFound") });
      return;
    }
    setConfirm({
      title: t("confirm.deleteSource"),
      message: t("confirm.deleteSourceMessage", {
        label: kindLabel,
        name: drawerName,
      }),
      confirmLabel: t("confirm.delete"),
      onConfirm: () => runDeleteEntries([entry]),
    });
  }, [drawerName, visible, kindLabel, runDeleteEntries, t, browsingSource]);

  const deleteAsset = requestDeleteAsset;

  const deleteFromSource = useCallback(
    async (entry: PlatformAssetEntry) => {
      setBusy(true);
      try {
        await invoke<string>(`cmd_${entry.kind}_delete`, {
          name: entry.name,
          project: activeProject,
        });
        if (drawerName === entry.name) closeDrawer();
        setToast({ kind: "ok", text: t("toast.deletedCount", { count: 1 }) });
        await refresh();
      } catch (e) {
        setToast({ kind: "err", text: t("toast.deleteFailed", { errors: String(e) }) });
      } finally {
        setBusy(false);
      }
    },
    [activeProject, drawerName, closeDrawer, refresh, t]
  );

  const deployOne = useCallback(
    async (entry: PlatformAssetEntry, plat: DeployPlatform) => {
      try {
        const r = await invoke<string>(`cmd_${entry.kind}_deploy`, {
          name: entry.name,
          project: activeProject,
          to: plat,
        });
        setToast({ kind: "ok", text: r });
        await refresh();
      } catch (e) {
        setToast({ kind: "err", text: t("toast.deployFailed", { error: e }) });
      }
    },
    [activeProject, refresh, t]
  );

  const retractOne = useCallback(
    async (entry: PlatformAssetEntry, plat: DeployPlatform) => {
      try {
        const r = await invoke<string>(`cmd_${entry.kind}_retract`, {
          name: entry.name,
          project: activeProject,
          from: plat,
        });
        setToast({ kind: "ok", text: r });
        await refresh();
      } catch (e) {
        setToast({ kind: "err", text: t("toast.retractFailed", { error: e }) });
      }
    },
    [activeProject, refresh, t]
  );

  const importFromPlatform = useCallback(
    async (name: string, kind: AssetKind) => {
      if (isSourcePlatform(activePlatform)) return;
      setBusy(true);
      try {
        const r = await invoke<string>(`cmd_${kind}_import`, {
          name,
          project: activeProject,
          fromPlatform: activePlatform,
        });
        setToast({ kind: "ok", text: t("toast.importSuccess", { message: r }) });
        await refreshView(false);
      } catch (e) {
        setToast({ kind: "err", text: t("toast.importFailed", { error: e }) });
      } finally {
        setBusy(false);
      }
    },
    [activePlatform, activeProject, refreshView, t]
  );

  const handlePlatformToggle = useCallback(
    (entry: PlatformAssetEntry, plat: Platform) => {
      if (plat === "aiconfig") {
        const state = entry.states.aiconfig;
        if (canRetract(state)) {
          void deleteFromSource(entry);
        } else if (canDeploy(state) && !isSourcePlatform(activePlatform)) {
          void importFromPlatform(entry.name, entry.kind);
        }
        return;
      }
      const deployPlat = plat as DeployPlatform;
      if (issueMap.has(`${deployPlat}:${entry.kind}`)) return;
      const state = entry.states[deployPlat];
      if (canRetract(state)) {
        void retractOne(entry, deployPlat);
      } else if (canDeploy(state)) {
        void deployOne(entry, deployPlat);
      }
    },
    [issueMap, retractOne, deployOne, deleteFromSource, importFromPlatform, activePlatform]
  );

  const togglePlatformBrowse = useCallback((plat: Platform) => {
    setActivePlatform(plat);
  }, []);

  const toggleSelect = useCallback((key: string, checked: boolean) => {
    setSelectedKeys((prev) => {
      const next = new Set(prev);
      if (checked) next.add(key);
      else next.delete(key);
      return next;
    });
  }, []);

  const toggleSelectAll = useCallback(() => {
    if (allVisibleSelected) {
      setSelectedKeys(new Set());
    } else {
      setSelectedKeys(new Set(visible.map(entryKey)));
    }
  }, [allVisibleSelected, visible]);

  const batchDeploy = useCallback(async () => {
    if (selectedEntries.length === 0) return;
    setBusy(true);
    try {
      if (browsingSource) {
        for (const entry of selectedEntries) {
          for (const p of DEPLOY_PLATFORMS) {
            if (canDeploy(entry.states[p]) && !issueMap.has(`${p}:${entry.kind}`)) {
              try {
                await invoke<string>(`cmd_${entry.kind}_deploy`, {
                  name: entry.name,
                  project: activeProject,
                  to: p,
                });
              } catch (e) {
                setToast({
                  kind: "err",
                  text: t("toast.batchDeployItemFailed", {
                    name: entry.name,
                    platform: p,
                    error: e,
                  }),
                });
              }
            }
          }
        }
      } else if (!isSourcePlatform(activePlatform)) {
        const plat = activePlatform as DeployPlatform;
        for (const entry of selectedEntries) {
          if (issueMap.has(`${plat}:${entry.kind}`)) continue;
          try {
            if (!hasSourceEntry(entry)) {
              await invoke<string>(`cmd_${entry.kind}_import`, {
                name: entry.name,
                project: activeProject,
                fromPlatform: activePlatform,
              });
            }
            await invoke<string>(`cmd_${entry.kind}_deploy`, {
              name: entry.name,
              project: activeProject,
              to: plat,
            });
          } catch (e) {
            setToast({
              kind: "err",
              text: t("toast.batchDeployItemFailed", {
                name: entry.name,
                platform: plat,
                error: e,
              }),
            });
          }
        }
      }
      await refreshView(false);
      setToast({
        kind: "ok",
        text: t("toast.batchDeployed", { count: selectedEntries.length }),
      });
    } finally {
      setBusy(false);
    }
  }, [selectedEntries, browsingSource, activePlatform, activeProject, issueMap, refreshView, t]);

  const batchRetract = useCallback(async () => {
    if (selectedEntries.length === 0) return;
    setBusy(true);
    try {
      if (browsingSource) {
        for (const entry of selectedEntries) {
          for (const p of DEPLOY_PLATFORMS) {
            if (canRetract(entry.states[p])) {
              try {
                await invoke<string>(`cmd_${entry.kind}_retract`, {
                  name: entry.name,
                  project: activeProject,
                  from: p,
                });
              } catch (e) {
                setToast({
                  kind: "err",
                  text: t("toast.batchRetractItemFailed", {
                    name: entry.name,
                    platform: p,
                    error: e,
                  }),
                });
              }
            }
          }
        }
      } else if (!isSourcePlatform(activePlatform)) {
        const plat = activePlatform as DeployPlatform;
        for (const entry of selectedEntries) {
          if (!canRetract(entry.states[plat])) continue;
          try {
            await invoke<string>(`cmd_${entry.kind}_retract`, {
              name: entry.name,
              project: activeProject,
              from: plat,
            });
          } catch (e) {
            setToast({
              kind: "err",
              text: t("toast.batchRetractItemFailed", {
                name: entry.name,
                platform: plat,
                error: e,
              }),
            });
          }
        }
      }
      await refreshView(false);
      setToast({
        kind: "ok",
        text: t("toast.batchRetracted", { count: selectedEntries.length }),
      });
    } finally {
      setBusy(false);
    }
  }, [selectedEntries, browsingSource, activePlatform, activeProject, refreshView, t]);

  const runBatchDelete = useCallback(async () => {
    setBusy(true);
    try {
      if (browsingSource) {
        await runDeleteEntries(selectedEntries);
      } else if (!isSourcePlatform(activePlatform)) {
        const plat = activePlatform as DeployPlatform;
        for (const entry of selectedEntries) {
          try {
            if (hasSourceEntry(entry)) {
              await invoke<string>(`cmd_${entry.kind}_delete`, {
                name: entry.name,
                project: activeProject,
              });
            } else if (canRetract(entry.states[plat])) {
              await invoke<string>(`cmd_${entry.kind}_retract`, {
                name: entry.name,
                project: activeProject,
                from: plat,
              });
            }
          } catch (e) {
            setToast({ kind: "err", text: t("toast.deleteFailed", { errors: String(e) }) });
          }
        }
        await refreshView(false);
        setToast({
          kind: "ok",
          text: t("toast.batchDeleted", { count: selectedEntries.length }),
        });
      }
    } finally {
      setBusy(false);
    }
  }, [selectedEntries, browsingSource, activePlatform, activeProject, runDeleteEntries, refreshView, t]);

  const requestBatchDelete = useCallback(() => {
    if (selectedEntries.length === 0) {
      setToast({ kind: "err", text: t("toast.selectRowsFirst") });
      return;
    }
    setConfirm({
      title: t("confirm.batchDelete"),
      message: browsingSource
        ? t("confirm.batchDeleteMessage", {
            count: selectedEntries.length,
            label: kindLabel,
          })
        : t("confirm.batchDeletePlatformMessage", {
            count: selectedEntries.length,
            platform: PLATFORM_NAME[activePlatform],
          }),
      confirmLabel: t("confirm.delete"),
      onConfirm: () => runBatchDelete(),
    });
  }, [selectedEntries, browsingSource, activePlatform, kindLabel, runBatchDelete, t]);

  const openBrowseFolder = useCallback(async () => {
    if (!browsePath) {
      setToast({ kind: "err", text: t("toast.noBrowsePath") });
      return;
    }
    try {
      await invoke("cmd_reveal_path", { path: browsePath });
    } catch (e) {
      setToast({ kind: "err", text: t("toast.openFolderFailed", { error: e }) });
    }
  }, [browsePath, t]);

  const submitRegisterProject = useCallback(
    async (name: string, rootPath: string) => {
      setRegisterProjectBusy(true);
      try {
        const project = await invoke<ProjectItem>("cmd_projects_add", {
          name,
          rootPath,
        });
        setProjects((p) => [...p, project]);
        setRegisterProjectOpen(false);
        setToast({ kind: "ok", text: t("toast.projectRegistered", { name: project.name }) });
      } catch (e) {
        setToast({ kind: "err", text: t("toast.registerProjectFailed", { error: e }) });
      } finally {
        setRegisterProjectBusy(false);
      }
    },
    [t]
  );

  const runRemoveProject = useCallback(
    async (name: string) => {
      setBusy(true);
      try {
        await invoke("cmd_projects_remove", { name });
        setProjects((p) => p.filter((q) => q.name !== name));
        if (activeProject === name) setActiveProject("user-global");
        setToast({ kind: "ok", text: t("toast.projectRemoved", { name }) });
      } catch (e) {
        setToast({ kind: "err", text: t("toast.removeProjectFailed", { error: e }) });
      } finally {
        setBusy(false);
        setConfirm(null);
      }
    },
    [activeProject, t]
  );

  const requestRemoveProject = useCallback(
    (name: string) => {
      setConfirm({
        title: t("confirm.removeProject"),
        message: t("confirm.removeProjectMessage", { name }),
        confirmLabel: t("confirm.removeProject"),
        onConfirm: () => runRemoveProject(name),
      });
    },
    [runRemoveProject, t]
  );

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    setSelectedKeys((prev) => {
      const next = new Set([...prev].filter((k) => visibleKeys.has(k)));
      return next.size === prev.size ? prev : next;
    });
  }, [visibleKeys]);

  const batchDisabled = loading || busy || selectedEntries.length === 0;
  const doctorIssueCount = doctor
    ? doctor.broken + doctor.wrong_source + doctor.wrong_type
    : 0;

  const platformPathMap = useMemo(
    () => new Map(platformKindPaths.map((r) => [r.platform, r])),
    [platformKindPaths]
  );

  const platformKindUnsupported =
    !isSourcePlatform(activePlatform) &&
    !!issueMap.get(`${activePlatform}:${activeKind}`);

  const toolbarTitle = `${kindLabel} · ${PLATFORM_NAME[activePlatform]} · ${
    activeProject === "user-global" ? t("nav.userGlobal") : activeProject
  }`;

  return (
    <div className="app">
      <header className="topbar">
        <span className="name">{t("app.name")}</span>
        <span className="branch">{t("app.version")}</span>
        <span className="status">
          <span className="dot down" />
          {t("app.daemonStopped")}
        </span>
        <div className="spacer" />
        <LanguageSwitcher />
      </header>

      <div className="main">
        <aside className="sidebar">
          <section className="sidebar-section sidebar-projects">
            <h2>{t("nav.projects")}</h2>
            <ul className="project-list">
              <li
                className={activeProject === "user-global" ? "active" : ""}
                onClick={() => setActiveProject("user-global")}
              >
                {t("nav.userGlobal")}
              </li>
              {projects.map((p) => (
                <li
                  key={p.id || p.name}
                  className={activeProject === p.name ? "active" : ""}
                  onClick={() => setActiveProject(p.name)}
                  title={p.root_path}
                >
                  <span className="project-name">{p.name}</span>
                  <span className="badge">.</span>
                  <span
                    className="remove"
                    onClick={(e) => {
                      e.stopPropagation();
                      requestRemoveProject(p.name);
                    }}
                    title={t("nav.removeProject", { name: p.name })}
                  >
                    ×
                  </span>
                </li>
              ))}
              <li
                className="add"
                onClick={() => setRegisterProjectOpen(true)}
                title={t("nav.registerProjectTitle")}
              >
                {t("nav.registerProject")}
              </li>
            </ul>
          </section>

          <section className="sidebar-section sidebar-assets">
            <h2>{t("nav.assets")}</h2>
            <ul>
              {ASSET_KINDS.map((k) => (
                <li
                  key={k}
                  className={activeKind === k ? "active" : ""}
                  onClick={() => setActiveKind(k)}
                >
                  {assetKindLabel(t, k)}
                  {issueMap.has(`cursor:${k}`) || issueMap.has(`codex:${k}`) ? (
                    <span className="badge">⚠</span>
                  ) : null}
                </li>
              ))}
            </ul>
          </section>

          <section className="sidebar-section sidebar-platforms">
            <h2>{t("nav.platforms")}</h2>
            <ul className="platform-list">
              {ALL_PLATFORMS.map((p) => {
                const pathInfo = platformPathMap.get(p);
                const pathHint = pathInfo?.path
                  ? pathInfo.supported
                    ? pathInfo.path
                    : t("platformView.unsupportedOnPlatform")
                  : "";
                return (
                  <li
                    key={p}
                    className={`platform-item${activePlatform === p ? " active" : ""}${
                      pathInfo && !pathInfo.supported ? " unsupported" : ""
                    }`}
                    title={
                      pathHint
                        ? `${PLATFORM_NAME[p]} · ${pathHint}`
                        : PLATFORM_NAME[p]
                    }
                    onClick={() => togglePlatformBrowse(p)}
                  >
                    <img
                      src={PLATFORM_FAVICON[p]}
                      alt=""
                      className="platform-item-icon"
                      draggable={false}
                    />
                    <span className="platform-item-name">{PLATFORM_NAME[p]}</span>
                  </li>
                );
              })}
            </ul>
          </section>
        </aside>

        <main className="content">
          <div className="toolbar">
            <label className="toolbar-check" title={t("toolbar.selectAllTitle")}>
              <input
                type="checkbox"
                checked={allVisibleSelected && visible.length > 0}
                onChange={toggleSelectAll}
                disabled={visible.length === 0 || loading}
              />
            </label>
            <div className="toolbar-body">
              <div className="toolbar-row">
                <h3 className="toolbar-title">{toolbarTitle}</h3>
                <div className="toolbar-actions">
                  <button
                    className="toolbar-btn"
                    disabled={batchDisabled}
                    title={
                      browsingSource
                        ? t("toolbar.batchDeploy")
                        : t("toolbar.batchDeployPlatform", {
                            platform: PLATFORM_NAME[activePlatform],
                          })
                    }
                    onClick={() => void batchDeploy()}
                  >
                    {t("toolbar.batchDeploy")}
                  </button>
                  <button
                    className="toolbar-btn"
                    disabled={batchDisabled}
                    title={
                      browsingSource
                        ? t("toolbar.batchRetract")
                        : t("toolbar.batchRetractPlatform", {
                            platform: PLATFORM_NAME[activePlatform],
                          })
                    }
                    onClick={() => void batchRetract()}
                  >
                    {t("toolbar.batchRetract")}
                  </button>
                  <button
                    className="toolbar-btn toolbar-btn-warn"
                    disabled={batchDisabled}
                    title={t("toolbar.batchDeleteTitle")}
                    onClick={requestBatchDelete}
                  >
                    {t("toolbar.batchDelete")}
                  </button>
                </div>
              </div>
              <div className="toolbar-row toolbar-row-sub">
                {browsePath ? (
                  <code className="toolbar-path" title={browsePath}>
                    {browsePath}
                  </code>
                ) : (
                  <span className="toolbar-path toolbar-path-empty" />
                )}
                <button
                  className="toolbar-btn"
                  disabled={loading || busy}
                  title={
                    loading ? t("toolbar.refreshing") : t("toolbar.refreshTitle")
                  }
                  onClick={() => void refreshView(true)}
                >
                  {t("toolbar.refresh")}
                </button>
                <button
                  className="toolbar-btn"
                  disabled={!browsePath || loading || busy}
                  title={t("toolbar.openFolderTitle")}
                  onClick={() => void openBrowseFolder()}
                >
                  {t("toolbar.openFolder")}
                </button>
                <span className="toolbar-meta">
                  {t("toolbar.items", { count: visible.length })}
                  {selectedEntries.length > 0
                    ? t("toolbar.selected", { count: selectedEntries.length })
                    : ""}
                  {loading || busy ? t("toolbar.processing") : ""}
                </span>
              </div>
            </div>
          </div>

          <div className="content-pane">
            {platformList === null ? (
              <div className="empty">{t("empty.loading")}</div>
            ) : visible.length === 0 ? (
              <div className="empty">
                {platformKindUnsupported ? (
                  t("platformView.emptyUnsupported", {
                    platform: PLATFORM_NAME[activePlatform],
                    label: kindLabel,
                  })
                ) : (
                  t("platformView.empty", { label: kindLabel })
                )}
              </div>
            ) : (
              <div className="asset-list">
                {visible.map((entry) => {
                  const key = entryKey(entry);
                  const canOpen = hasSourceEntry(entry);
                  return (
                    <AssetRow
                      key={key}
                      entry={entry}
                      loading={loading || busy}
                      checked={selectedKeys.has(key)}
                      onCheckedChange={(checked) => toggleSelect(key, checked)}
                      selected={drawerName === entry.name}
                      onOpen={canOpen ? () => void openAsset(entry.name) : undefined}
                      issueReasonFor={(plat) => issueReasonFor(entry, plat)}
                      onPlatformToggle={(plat) => handlePlatformToggle(entry, plat)}
                    />
                  );
                })}
              </div>
            )}

            {drawerName && browsingSource ? (
              <AssetDrawer
                kind={activeKind}
                name={drawerName}
                detail={drawerDetail}
                loading={drawerLoading || busy}
                editing={drawerEditing}
                draft={drawerDraft}
                onClose={closeDrawer}
                onDraftChange={setDrawerDraft}
                onEdit={() => setDrawerEditing(true)}
                onCancelEdit={() => {
                  setDrawerEditing(false);
                  setDrawerDraft(drawerDetail?.content ?? "");
                }}
                onSave={() => void saveAsset()}
                onDelete={deleteAsset}
              />
            ) : null}
          </div>
        </main>
      </div>

      <footer className="statusbar">
        <span>
          {doctor
            ? t("statusbar.doctor", { count: doctorIssueCount })
            : t("statusbar.doctorLoading")}
        </span>
        <span>{t("statusbar.secrets", { count: doctor?.missing_secrets.length ?? 0 })}</span>
        <span>{t("statusbar.projects", { count: projects.length })}</span>
        <span>{t("statusbar.broken", { count: 0 })}</span>
      </footer>

      {toast ? (
        <div className={`toast ${toast.kind}`} onClick={() => setToast(null)}>
          {toast.text}
        </div>
      ) : null}

      {confirm ? (
        <ConfirmModal
          title={confirm.title}
          message={confirm.message}
          confirmLabel={confirm.confirmLabel}
          busy={busy}
          onCancel={() => {
            if (!busy) setConfirm(null);
          }}
          onConfirm={() => void confirm.onConfirm()}
        />
      ) : null}

      {registerProjectOpen ? (
        <RegisterProjectModal
          busy={registerProjectBusy}
          onCancel={() => {
            if (!registerProjectBusy) setRegisterProjectOpen(false);
          }}
          onSubmit={(name, rootPath) => void submitRegisterProject(name, rootPath)}
        />
      ) : null}
    </div>
  );
}
