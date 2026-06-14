/**
 * ai-config GUI 主组件 — 统一列表 + 批量操作 + 资产编辑抽屉
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Trans, useTranslation } from "react-i18next";
import { AssetDrawer } from "./AssetDrawer";
import { AssetRow } from "./AssetRow";
import { ConfirmModal } from "./ConfirmModal";
import { RegisterProjectModal } from "./RegisterProjectModal";
import { LanguageSwitcher } from "./LanguageSwitcher";
import { assetKindLabel, kindDirName } from "./i18n/labels";
import {
  ASSET_KINDS,
  canDeploy,
  canRetract,
  PLATFORMS,
  type AssetDetail,
  type AssetEntry,
  type AssetKind,
  type AssetList,
  type DoctorSummary,
  type Platform,
  type ProjectItem,
} from "./types";

function entryKey(entry: AssetEntry): string {
  return `${entry.kind}:${entry.name}`;
}

export function App() {
  const { t } = useTranslation();
  const [activeProject, setActiveProject] = useState<string>("user-global");
  const [activeKind, setActiveKind] = useState<AssetKind>("skill");
  const [projects, setProjects] = useState<ProjectItem[]>([]);
  const [list, setList] = useState<AssetList | null>(null);
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
    const myReq = ++reqIdRef.current;
    setLoading(true);
    invoke<AssetList>("cmd_list", { project: activeProject })
      .then((r) => {
        if (reqIdRef.current === myReq) setList(r);
      })
      .catch((e) => {
        if (reqIdRef.current === myReq) {
          setToast({ kind: "err", text: t("toast.listFailed", { error: e }) });
          setList(null);
        }
      })
      .finally(() => {
        if (reqIdRef.current === myReq) setLoading(false);
      });
  }, [activeProject]);

  const visible = useMemo(
    () => list?.entries.filter((e) => e.kind === activeKind) ?? [],
    [list, activeKind]
  );

  const visibleKeys = useMemo(
    () => new Set(visible.map(entryKey)),
    [visible]
  );

  const selectedEntries = useMemo(
    () => visible.filter((e) => selectedKeys.has(entryKey(e))),
    [visible, selectedKeys]
  );

  const deleteTargets = selectedEntries;

  const allVisibleSelected =
    visible.length > 0 && visible.every((e) => selectedKeys.has(entryKey(e)));

  const emptyAssetRoot = useMemo(() => {
    if (activeProject === "user-global") return null;
    const p = projects.find((q) => q.name === activeProject);
    if (!p?.root_path) return null;
    return `${p.root_path}/.ai-config`;
  }, [activeProject, projects]);

  const issues = doctor?.platform_capability_issues ?? [];
  const issueMap = new Map(issues.map((i) => [`${i.platform}:${i.kind}`, i.reason]));

  const issueReasonFor = useCallback(
    (entry: AssetEntry, plat: Platform) =>
      issueMap.get(`${plat}:${entry.kind}`),
    [issueMap]
  );

  const refresh = useCallback(async () => {
    const myReq = ++reqIdRef.current;
    try {
      const r = await invoke<AssetList>("cmd_list", { project: activeProject });
      if (reqIdRef.current === myReq) setList(r);
      return r;
    } catch (e) {
      if (reqIdRef.current === myReq) {
        setToast({ kind: "err", text: t("toast.refreshFailed", { error: e }) });
      }
      return null;
    }
  }, [activeProject, t]);

  const closeDrawer = useCallback(() => {
    setDrawerName(null);
    setDrawerDetail(null);
    setDrawerEditing(false);
    setDrawerDraft("");
  }, []);

  const reloadDrawerIfOpen = useCallback(
    async (listResult: AssetList | null) => {
      const name = drawerNameRef.current;
      if (!name || !listResult) return;

      const stillExists = listResult.entries.some(
        (e) => e.kind === activeKind && e.name === name
      );
      if (!stillExists) {
        closeDrawer();
        setToast({
          kind: "err",
          text: t("toast.loadAssetFailed", {
            label: assetKindLabel(t, activeKind),
            error: t("toast.assetRemovedExternally"),
          }),
        });
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
  }, [activeKind, activeProject]);

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
    async (entries: AssetEntry[]) => {
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
    if (!drawerName) return;
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
  }, [drawerName, visible, kindLabel, runDeleteEntries, t]);

  const deleteAsset = requestDeleteAsset;

  const deployOne = useCallback(
    async (entry: AssetEntry, plat: Platform) => {
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
    async (entry: AssetEntry, plat: Platform) => {
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

  const handlePlatformToggle = useCallback(
    (entry: AssetEntry, plat: Platform) => {
      if (issueMap.has(`${plat}:${entry.kind}`)) return;
      const state = entry.states[plat];
      if (canRetract(state)) {
        void retractOne(entry, plat);
      } else if (canDeploy(state)) {
        void deployOne(entry, plat);
      }
    },
    [issueMap, retractOne, deployOne]
  );

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
    setLoading(true);
    try {
      for (const entry of selectedEntries) {
        for (const p of PLATFORMS) {
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
      await refresh();
      setToast({
        kind: "ok",
        text: t("toast.batchDeployed", { count: selectedEntries.length }),
      });
    } finally {
      setLoading(false);
    }
  }, [selectedEntries, activeProject, issueMap, refresh, t]);

  const batchRetract = useCallback(async () => {
    if (selectedEntries.length === 0) return;
    setLoading(true);
    try {
      for (const entry of selectedEntries) {
        for (const p of PLATFORMS) {
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
      await refresh();
      setToast({
        kind: "ok",
        text: t("toast.batchRetracted", { count: selectedEntries.length }),
      });
    } finally {
      setLoading(false);
    }
  }, [selectedEntries, activeProject, refresh, t]);

  const requestBatchDelete = useCallback(() => {
    if (deleteTargets.length === 0) {
      setToast({ kind: "err", text: t("toast.selectRowsFirst") });
      return;
    }
    setConfirm({
      title: t("confirm.batchDelete"),
      message: t("confirm.batchDeleteMessage", {
        count: deleteTargets.length,
        label: kindLabel,
      }),
      confirmLabel: t("confirm.delete"),
      onConfirm: () => runDeleteEntries(deleteTargets),
    });
  }, [deleteTargets, kindLabel, runDeleteEntries, t]);

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
  const batchDeleteDisabled = loading || busy;
  const doctorIssueCount = doctor
    ? doctor.broken + doctor.wrong_source + doctor.wrong_type
    : 0;

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
        <aside className="column">
          <h2>{t("nav.projects")}</h2>
          <ul>
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
        </aside>

        <aside className="column">
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
            <h3>
              {kindLabel} · {activeProject === "user-global" ? t("nav.userGlobal") : activeProject}
            </h3>
            <span className="toolbar-meta">
              {t("toolbar.items", { count: visible.length })}
              {selectedEntries.length > 0
                ? t("toolbar.selected", { count: selectedEntries.length })
                : ""}
              {loading || busy ? t("toolbar.processing") : ""}
            </span>
            <div className="spacer" />
            <button
              disabled={loading || busy}
              title={t("toolbar.refreshTitle")}
              onClick={() => void refreshView(true)}
            >
              {loading ? t("toolbar.refreshing") : t("toolbar.refresh")}
            </button>
            <button className="primary" disabled={batchDisabled} onClick={() => void batchDeploy()}>
              {t("toolbar.batchDeploy")}
            </button>
            <button disabled={batchDisabled} onClick={() => void batchRetract()}>
              {t("toolbar.batchRetract")}
            </button>
            <button
              className="danger"
              disabled={batchDeleteDisabled}
              title={t("toolbar.batchDeleteTitle")}
              onClick={requestBatchDelete}
            >
              {t("toolbar.batchDelete")}
            </button>
          </div>

          <div className="content-pane">
            {list === null ? (
              <div className="empty">{t("empty.loading")}</div>
            ) : visible.length === 0 ? (
              <div className="empty">
                {activeKind === "mcp" ? (
                  emptyAssetRoot ? (
                    <Trans
                      i18nKey="empty.mcp"
                      values={{ assetRoot: emptyAssetRoot }}
                      components={{ code: <code /> }}
                    />
                  ) : (
                    t("empty.mcpGlobal")
                  )
                ) : emptyAssetRoot ? (
                  <Trans
                    i18nKey="empty.assets"
                    values={{
                      assetRoot: emptyAssetRoot,
                      kindDir: kindDirName(t, activeKind),
                    }}
                    components={{ code: <code /> }}
                  />
                ) : (
                  <Trans
                    i18nKey="empty.assetsGlobal"
                    values={{ kindDir: kindDirName(t, activeKind) }}
                    components={{ code: <code /> }}
                  />
                )}
              </div>
            ) : (
              <div className="asset-list">
                {visible.map((entry) => {
                  const key = entryKey(entry);
                  return (
                    <AssetRow
                      key={key}
                      entry={entry}
                      loading={loading || busy}
                      checked={selectedKeys.has(key)}
                      onCheckedChange={(checked) => toggleSelect(key, checked)}
                      selected={drawerName === entry.name}
                      onOpen={() => void openAsset(entry.name)}
                      issueReasonFor={(plat) => issueReasonFor(entry, plat)}
                      onPlatformToggle={(plat) => handlePlatformToggle(entry, plat)}
                    />
                  );
                })}
              </div>
            )}

            {drawerName ? (
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
        <span>{t("statusbar.broken", { count: list?.broken_links ?? 0 })}</span>
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
