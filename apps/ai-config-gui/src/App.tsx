/**
 * ai-config GUI 主组件 — 统一列表 + 批量操作 + 资产编辑抽屉
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AssetDrawer } from "./AssetDrawer";
import { AssetRow } from "./AssetRow";
import { ConfirmModal } from "./ConfirmModal";
import {
  ASSET_KINDS,
  ASSET_LABEL,
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

  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [drawerName, setDrawerName] = useState<string | null>(null);
  const [drawerDetail, setDrawerDetail] = useState<AssetDetail | null>(null);
  const [drawerEditing, setDrawerEditing] = useState(false);
  const [drawerDraft, setDrawerDraft] = useState("");
  const [drawerLoading, setDrawerLoading] = useState(false);

  const reqIdRef = useRef(0);

  useEffect(() => {
    invoke<DoctorSummary>("cmd_doctor").then(setDoctor).catch(console.error);
    invoke<ProjectItem[]>("cmd_projects_list")
      .then(setProjects)
      .catch((e) => setToast({ kind: "err", text: `加载项目列表失败: ${e}` }));
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
          setToast({ kind: "err", text: `cmd_list 失败: ${e}` });
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

  /** 批量删除仅认勾选行 */
  const deleteTargets = selectedEntries;

  const allVisibleSelected =
    visible.length > 0 && visible.every((e) => selectedKeys.has(entryKey(e)));

  const issues = doctor?.platform_capability_issues ?? [];
  const issueMap = new Map(issues.map((i) => [`${i.platform}:${i.kind}`, i.reason]));

  const issueReasonFor = useCallback(
    (entry: AssetEntry, plat: Platform) =>
      issueMap.get(`${plat}:${entry.kind}`),
    [issueMap]
  );

  const refresh = useCallback(async () => {
    const myReq = reqIdRef.current;
    try {
      const r = await invoke<AssetList>("cmd_list", { project: activeProject });
      if (reqIdRef.current === myReq) setList(r);
    } catch (e) {
      setToast({ kind: "err", text: `刷新失败: ${e}` });
    }
  }, [activeProject]);

  const closeDrawer = useCallback(() => {
    setDrawerName(null);
    setDrawerDetail(null);
    setDrawerEditing(false);
    setDrawerDraft("");
  }, []);

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
        setToast({ kind: "err", text: `加载 ${ASSET_LABEL[activeKind]} 失败: ${e}` });
        setDrawerName(null);
      } finally {
        setDrawerLoading(false);
      }
    },
    [activeKind, activeProject]
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
      setToast({ kind: "err", text: `保存失败: ${e}` });
    } finally {
      setDrawerLoading(false);
    }
  }, [drawerName, drawerDraft, activeKind, activeProject, refresh]);

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
          setToast({ kind: "ok", text: `已删除 ${ok} 个源文件` });
        } else if (ok === 0) {
          setToast({
            kind: "err",
            text: `删除失败: ${failed.join("；")}`,
          });
        } else {
          setToast({
            kind: "err",
            text: `部分删除: 成功 ${ok}，失败 ${failed.length}（${failed.join("；")}）`,
          });
        }
      } finally {
        setBusy(false);
        setConfirm(null);
      }
    },
    [activeProject, drawerName, closeDrawer, refresh]
  );

  const requestDeleteAsset = useCallback(() => {
    if (!drawerName) return;
    const entry = visible.find((e) => e.name === drawerName);
    if (!entry) {
      setToast({ kind: "err", text: "未找到要删除的条目" });
      return;
    }
    setConfirm({
      title: "删除源文件",
      message: `将永久删除 ${ASSET_LABEL[activeKind]}「${drawerName}」`,
      confirmLabel: "删除",
      onConfirm: () => runDeleteEntries([entry]),
    });
  }, [drawerName, visible, activeKind, runDeleteEntries]);

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
        setToast({ kind: "err", text: `下发失败: ${e}` });
      }
    },
    [activeProject, refresh]
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
        setToast({ kind: "err", text: `收回失败: ${e}` });
      }
    },
    [activeProject, refresh]
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
                text: `${entry.name} → ${p} 失败: ${e}`,
              });
            }
          }
        }
      }
      await refresh();
      setToast({ kind: "ok", text: `已批量下发 ${selectedEntries.length} 项` });
    } finally {
      setLoading(false);
    }
  }, [selectedEntries, activeProject, issueMap, refresh]);

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
                text: `${entry.name} ← ${p} 失败: ${e}`,
              });
            }
          }
        }
      }
      await refresh();
      setToast({ kind: "ok", text: `已批量收回 ${selectedEntries.length} 项` });
    } finally {
      setLoading(false);
    }
  }, [selectedEntries, activeProject, refresh]);

  const requestBatchDelete = useCallback(() => {
    if (deleteTargets.length === 0) {
      setToast({ kind: "err", text: "请先勾选左侧复选框（打开详情不会自动选中）" });
      return;
    }
    setConfirm({
      title: "批量删除",
      message: `将永久删除 ${deleteTargets.length} 个${ASSET_LABEL[activeKind]}源文件，并尽力收回各平台链接。`,
      confirmLabel: "删除",
      onConfirm: () => runDeleteEntries(deleteTargets),
    });
  }, [deleteTargets, activeKind, runDeleteEntries]);

  const addProject = useCallback(async () => {
    const name = window.prompt("项目名(在 store 中唯一):", "");
    if (!name) return;
    const rootPath = window.prompt("项目根目录绝对路径:", "");
    if (!rootPath) return;
    try {
      await invoke<ProjectItem>("cmd_projects_add", { name, rootPath });
      setProjects((p) => [...p, { id: 0, name, root_path: rootPath, registered_at: "" }]);
      setToast({ kind: "ok", text: `项目 \`${name}\` 已注册` });
    } catch (e) {
      setToast({ kind: "err", text: `注册项目失败: ${e}` });
    }
  }, []);

  const removeProject = useCallback(async (name: string) => {
    if (!window.confirm(`确认从本工具中移除项目 \`${name}\`?`)) return;
    try {
      await invoke("cmd_projects_remove", { name });
      setProjects((p) => p.filter((q) => q.name !== name));
      if (activeProject === name) setActiveProject("user-global");
      setToast({ kind: "ok", text: `项目 \`${name}\` 已移除` });
    } catch (e) {
      setToast({ kind: "err", text: `移除项目失败: ${e}` });
    }
  }, [activeProject]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 4000);
    return () => clearTimeout(t);
  }, [toast]);

  // 列表刷新后去掉已不存在的勾选
  useEffect(() => {
    setSelectedKeys((prev) => {
      const next = new Set([...prev].filter((k) => visibleKeys.has(k)));
      return next.size === prev.size ? prev : next;
    });
  }, [visibleKeys]);

  const batchDisabled = loading || busy || selectedEntries.length === 0;
  const batchDeleteDisabled = loading || busy;

  return (
    <div className="app">
      <header className="topbar">
        <span className="name">ai-config</span>
        <span className="branch">v0.1.0 · M3 (W9)</span>
        <span className="status">
          <span className="dot down" />
          daemon stopped
        </span>
      </header>

      <div className="main">
        <aside className="column">
          <h2>项目</h2>
          <ul>
            <li
              className={activeProject === "user-global" ? "active" : ""}
              onClick={() => setActiveProject("user-global")}
            >
              user-global
              <span className="badge">M3</span>
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
                    removeProject(p.name);
                  }}
                  title={`移除 ${p.name}`}
                >
                  ×
                </span>
              </li>
            ))}
            <li className="add" onClick={addProject} title="注册新项目">
              + 注册项目
            </li>
          </ul>
        </aside>

        <aside className="column">
          <h2>资产</h2>
          <ul>
            {ASSET_KINDS.map((k) => (
              <li
                key={k}
                className={activeKind === k ? "active" : ""}
                onClick={() => setActiveKind(k)}
              >
                {ASSET_LABEL[k]}
                {issueMap.has(`cursor:${k}`) || issueMap.has(`codex:${k}`) ? (
                  <span className="badge">⚠</span>
                ) : null}
              </li>
            ))}
          </ul>
        </aside>

        <main className="content">
          <div className="toolbar">
            <label className="toolbar-check" title="全选当前列表">
              <input
                type="checkbox"
                checked={allVisibleSelected && visible.length > 0}
                onChange={toggleSelectAll}
                disabled={visible.length === 0 || loading}
              />
            </label>
            <h3>
              {ASSET_LABEL[activeKind]} · {activeProject}
            </h3>
            <span className="toolbar-meta">
              {visible.length} 项
              {selectedEntries.length > 0
                ? ` · 已选 ${selectedEntries.length}`
                : ""}
              {loading || busy ? " (处理中…)" : ""}
            </span>
            <div className="spacer" />
            <button
              className="primary"
              disabled={batchDisabled}
              onClick={batchDeploy}
            >
              批量下发
            </button>
            <button disabled={batchDisabled} onClick={batchRetract}>
              批量收回
            </button>
            <button
              className="danger"
              disabled={batchDeleteDisabled}
              title="勾选左侧复选框后删除源文件"
              onClick={requestBatchDelete}
            >
              批量删除
            </button>
          </div>

          <div className="content-pane">
            {list === null ? (
              <div className="empty">加载中…</div>
            ) : visible.length === 0 ? (
              <div className="empty">
                {activeKind === "mcp" ? (
                  <>暂无 MCP server，可编辑 mcp.json 或添加</>
                ) : (
                  <>
                    暂无资产。在 <code>~/.ai-config/{activeKind}s/</code> 下添加文件后刷新。
                  </>
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
                      onPlatformToggle={(plat) =>
                        handlePlatformToggle(entry, plat)
                      }
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
                onSave={saveAsset}
                onDelete={deleteAsset}
              />
            ) : null}
          </div>
        </main>
      </div>

      <footer className="statusbar">
        <span>
          doctor:{" "}
          {doctor
            ? `${doctor.broken + doctor.wrong_source + doctor.wrong_type} 异常`
            : "..."}
        </span>
        <span>secrets: {doctor?.missing_secrets.length ?? 0} 缺</span>
        <span>projects: {projects.length} 注册</span>
        <span>broken: {list?.broken_links ?? 0}</span>
        <span className="right">Phase 3 · W9</span>
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
    </div>
  );
}
