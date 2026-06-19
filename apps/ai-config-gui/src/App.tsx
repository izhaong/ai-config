/**
 * ai-config GUI 主组件 — 统一列表 + 批量操作 + 资产编辑抽屉
 */

import { useMemoizedFn } from "ahooks";
import { useEffect, useRef, useState } from "react";

import { ConfirmModal } from "./components/feedback/ConfirmModal";
import { RegisterProjectModal } from "./components/feedback/RegisterProjectModal";
import { AssetListPane } from "./components/assets/AssetListPane";
import { AssetToolbar } from "./components/assets/AssetToolbar";
import { Toast } from "./components/feedback/Toast";
import { AppSidebar } from "./components/layout/AppSidebar";
import { AppStatusbar } from "./components/layout/AppStatusbar";
import { useAssetBrowser } from "./hooks/useAssetBrowser";
import { useAssetDrawer } from "./hooks/useAssetDrawer";
import { useAssetOperations } from "./hooks/useAssetOperations";
import { useConfirm } from "./hooks/useConfirm";
import { useProjects } from "./hooks/useProjects";
import { useToast } from "./hooks/useToast";
import type { PlatformAssetList } from "./types";

export function App() {
  const { toast, showToast, dismissToast } = useToast();
  const { confirm, requestConfirm, dismissConfirm } = useConfirm();

  const [activeProject, setActiveProject] = useState<string>("user-global");
  const [busy, setBusy] = useState(false);

  const drawerResetRef = useRef<(() => void) | null>(null);
  const reloadDrawerRef = useRef<
    (list: PlatformAssetList | null) => Promise<void>
  >(async () => {});

  const handleContextReset = useMemoizedFn(() => {
    drawerResetRef.current?.();
  });

  const handleReloadDrawer = useMemoizedFn((list: PlatformAssetList | null) => {
    return reloadDrawerRef.current(list);
  });

  const projects = useProjects({
    showToast,
    requestConfirm,
    dismissConfirm,
    setBusy,
    activeProject,
    onActiveProjectChange: setActiveProject,
  });

  const browser = useAssetBrowser({
    showToast,
    doctor: projects.doctor,
    activeProject,
    setActiveProject,
    onContextReset: handleContextReset,
    reloadDrawerIfOpen: handleReloadDrawer,
  });

  const drawer = useAssetDrawer({
    activeProject: browser.activeProject,
    activeKind: browser.activeKind,
    activePlatform: browser.activePlatform,
    visible: browser.visible,
    browsingSource: browser.browsingSource,
    refresh: browser.refresh,
    clearSelection: browser.clearSelection,
    showToast,
    requestConfirm,
    dismissConfirm,
    setBusy,
  });

  useEffect(() => {
    drawerResetRef.current = drawer.resetDrawer;
    reloadDrawerRef.current = drawer.reloadDrawerIfOpen;
  }, [drawer.resetDrawer, drawer.reloadDrawerIfOpen]);

  const ops = useAssetOperations({
    browser,
    drawer,
    showToast,
    requestConfirm,
    dismissConfirm,
    setBusy,
  });

  const handleDrawerCancelEdit = useMemoizedFn(() => {
    drawer.setDrawerEditing(false);
    drawer.setDrawerDraft(drawer.drawerDetail?.content ?? "");
  });

  return (
    <div className="app">
      <div className="main">
        <AppSidebar
          activeProject={browser.activeProject}
          onActiveProjectChange={browser.setActiveProject}
          projects={projects.projects}
          onRequestRemoveProject={projects.requestRemoveProject}
          onRegisterProject={() => projects.setRegisterProjectOpen(true)}
          activeKind={browser.activeKind}
          onActiveKindChange={browser.setActiveKind}
          activePlatform={browser.activePlatform}
          onPlatformBrowse={browser.togglePlatformBrowse}
          platformPathMap={browser.platformPathMap}
          issueMap={browser.issueMap}
        />

        <main className="content">
          <AssetToolbar
            activeKind={browser.activeKind}
            activePlatform={browser.activePlatform}
            activeProject={browser.activeProject}
            browsingSource={browser.browsingSource}
            allVisibleSelected={browser.allVisibleSelected}
            visibleCount={browser.visible.length}
            selectedCount={browser.selectedEntries.length}
            selectedEntries={browser.selectedEntries}
            browsePath={browser.browsePath}
            loading={browser.loading}
            busy={busy}
            issueReasonForKind={(plat) =>
              browser.issueMap.get(`${plat}:${browser.activeKind}`)
            }
            onToggleSelectAll={browser.toggleSelectAll}
            onBatchSyncPlatform={(plat) => void ops.batchSyncToPlatform(plat)}
            onBatchDelete={ops.requestBatchDelete}
            onOpenFolder={() => void ops.openBrowseFolder()}
          />

          <AssetListPane
            platformList={browser.platformList}
            visible={browser.visible}
            activeKind={browser.activeKind}
            activePlatform={browser.activePlatform}
            platformKindUnsupported={browser.platformKindUnsupported}
            loading={browser.loading}
            busy={busy}
            selectedKeys={browser.selectedKeys}
            drawerName={drawer.drawerName}
            drawerDetail={drawer.drawerDetail}
            drawerLoading={drawer.drawerLoading}
            drawerEditing={drawer.drawerEditing}
            drawerDraft={drawer.drawerDraft}
            drawerReadOnly={drawer.drawerReadOnly}
            canOpenEntry={browser.canOpenEntry}
            issueReasonFor={browser.issueReasonFor}
            onToggleSelect={browser.toggleSelect}
            onOpenAsset={(entry) => void drawer.openAsset(entry)}
            onPlatformToggle={ops.handlePlatformToggle}
            onDeleteEntry={ops.requestDeleteEntry}
            onCloseDrawer={drawer.closeDrawer}
            onDraftChange={drawer.setDrawerDraft}
            onEdit={() => drawer.setDrawerEditing(true)}
            onCancelEdit={handleDrawerCancelEdit}
            onSave={() => void drawer.saveAsset()}
            onDelete={drawer.deleteAsset}
          />
        </main>
      </div>

      <AppStatusbar
        doctor={projects.doctor}
        doctorIssueCount={projects.doctorIssueCount}
        projectCount={projects.projects.length}
        loading={browser.loading}
        busy={busy}
        onRefresh={() => void browser.refreshView(true)}
      />

      <Toast toast={toast} onDismiss={dismissToast} />

      {confirm ? (
        <ConfirmModal
          title={confirm.title}
          message={confirm.message}
          confirmLabel={confirm.confirmLabel}
          busy={busy}
          onCancel={() => {
            if (!busy) dismissConfirm();
          }}
          onConfirm={() => void confirm.onConfirm()}
        />
      ) : null}

      {projects.registerProjectOpen ? (
        <RegisterProjectModal
          busy={projects.registerProjectBusy}
          onCancel={() => {
            if (!projects.registerProjectBusy) {
              projects.setRegisterProjectOpen(false);
            }
          }}
          onSubmit={(name, rootPath) =>
            void projects.submitRegisterProject(name, rootPath)
          }
        />
      ) : null}
    </div>
  );
}
