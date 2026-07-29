/**
 * ai-config GUI 主组件 — 统一列表 + 批量操作 + 资产编辑抽屉
 */

import { useMemoizedFn } from "ahooks";
import { useEffect, useRef, useState } from "react";

import { CopyToProjectModal } from "./components/feedback/CopyToProjectModal";
import { ConfirmModal } from "./components/feedback/ConfirmModal";
import { ProjectionPlanModal } from "./components/feedback/ProjectionPlanModal";
import { ImportPlanModal } from "./components/feedback/ImportPlanModal";
import { UpdateModal } from "./components/feedback/UpdateModal";
import { AddSkillModal } from "./components/feedback/AddSkillModal";
import { AddMcpModal } from "./components/feedback/AddMcpModal";
import { MarketplaceSkillModal } from "./components/feedback/MarketplaceSkillModal";
import { RegisterProjectModal } from "./components/feedback/RegisterProjectModal";
import { AssetListPane } from "./components/assets/AssetListPane";
import { AssetToolbar } from "./components/assets/AssetToolbar";
import { Toast } from "./components/feedback/Toast";
import { AppSidebar } from "./components/layout/AppSidebar";
import { AppStatusbar } from "./components/layout/AppStatusbar";
import { useAppUpdater } from "./hooks/useAppUpdater";
import { useAssetBrowser } from "./hooks/useAssetBrowser";
import { useAssetDrawer } from "./hooks/useAssetDrawer";
import { useAssetOperations } from "./hooks/useAssetOperations";
import { useConfirm } from "./hooks/useConfirm";
import { useGitSync } from "./hooks/useGitSync";
import { useProjects } from "./hooks/useProjects";
import { useToast } from "./hooks/useToast";
import { PLATFORM_NAME } from "./platformIcons";
import type { PlatformAssetList } from "./types";
import { rowSyncLeadSlot } from "./utils/rowSyncLeadSlot";

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

  const gitSync = useGitSync({ showToast });

  const appUpdater = useAppUpdater({ showToast });

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
    projects: projects.projects,
    showToast,
    requestConfirm,
    dismissConfirm,
    setBusy,
  });

  const handleDrawerCancelEdit = useMemoizedFn(() => {
    drawer.setDrawerEditing(false);
    drawer.setDrawerDraft(drawer.drawerDetail?.content ?? "");
  });

  const leadSlot = rowSyncLeadSlot(browser.activeKind, browser.browsingSource);

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
          <div className="content-pane">
            <div
              className={`content-pane-scroll${drawer.drawerName ? " is-drawer-open" : ""}`}
            >
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
                onBatchUpdate={() => ops.batchUpdateEntries()}
                onBatchCopyToProject={() => ops.openCopyToProject()}
                onBatchDelete={ops.requestBatchDelete}
                onOpenFolder={() => void ops.openBrowseFolder()}
                onAddSkill={() => ops.openAddSkill()}
                onAddMcp={() => ops.openAddMcp()}
                onAddMarketplace={() => ops.openAddMarketplace()}
              />

              <AssetListPane
                platformList={browser.platformList}
                visible={browser.visible}
                activeKind={browser.activeKind}
                activePlatform={browser.activePlatform}
                leadSlot={leadSlot}
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
                drawerHookEntry={drawer.drawerHookEntry}
                issueReasonFor={browser.issueReasonFor}
                onToggleSelect={browser.toggleSelect}
                onOpenAsset={(entry) => void drawer.openAsset(entry)}
                onPlatformToggle={ops.handlePlatformToggle}
                onUpdateEntry={ops.handleUpdateEntry}
                onDeleteEntry={ops.requestDeleteEntry}
                onHookLifecyclePairToggle={(entry, plan) =>
                  void ops.handleHookLifecyclePairToggle(entry, plan)
                }
                onCloseDrawer={drawer.closeDrawer}
                onDraftChange={drawer.setDrawerDraft}
                onEdit={() => drawer.setDrawerEditing(true)}
                onCancelEdit={handleDrawerCancelEdit}
                onSave={() => void drawer.saveAsset()}
                onDelete={drawer.deleteAsset}
              />
            </div>
          </div>
        </main>
      </div>

      <AppStatusbar
        doctor={projects.doctor}
        doctorIssueCount={projects.doctorIssueCount}
        projectCount={projects.projects.length}
        loading={browser.loading}
        busy={busy}
        onRefresh={() => void browser.refreshView(true)}
        gitStatusLabel={gitSync.statusLabel}
        gitSettings={{
          assetRoot: gitSync.assetRoot,
          remoteDraft: gitSync.remoteDraft,
          branchDraft: gitSync.branchDraft,
          onRemoteChange: gitSync.setRemoteDraft,
          onBranchChange: gitSync.setBranchDraft,
          onSaveGitConfig: () => void gitSync.saveConfig(),
          onGitSync: () => void gitSync.sync(),
          onGitPull: () => void gitSync.pull(),
          onGitPush: () => void gitSync.push(),
          gitBusy: gitSync.busy,
          hasRemote: gitSync.hasRemote,
          onCheckUpdate: () => void appUpdater.checkForUpdate(true),
        }}
      />

      <Toast toast={toast} onDismiss={dismissToast} />

      <UpdateModal
          open={appUpdater.modalOpen && !!appUpdater.update}
          currentVersion={appUpdater.update?.currentVersion ?? ""}
          newVersion={appUpdater.update?.version ?? ""}
          notes={appUpdater.update?.body ?? undefined}
          installing={appUpdater.installing}
          progress={appUpdater.progress}
          onInstall={() => void appUpdater.installUpdate()}
          onLater={appUpdater.dismissModal}
        />

      <ConfirmModal
          open={!!confirm}
          title={confirm?.title ?? ""}
          message={confirm?.message ?? ""}
          confirmLabel={confirm?.confirmLabel}
          busy={busy}
          onCancel={() => {
            if (!busy) dismissConfirm();
          }}
          onConfirm={() => void confirm?.onConfirm()}
        />

      <ProjectionPlanModal
        open={!!ops.projectionReview}
        review={ops.projectionReview?.review ?? null}
        retract={ops.projectionReview?.retract ?? false}
        busy={busy}
        onCancel={ops.dismissProjectionReview}
        onApply={() => void ops.confirmProjectionReview()}
      />

      <ImportPlanModal
        open={!!ops.importReview}
        plan={ops.importReview?.plan ?? null}
        busy={busy}
        onCancel={ops.dismissImportReview}
        onApply={() => void ops.confirmImportReview()}
      />

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

      {ops.copyToProjectOpen ? (
        <CopyToProjectModal
          key={`${browser.activeProject}:${browser.selectedEntries.map((e) => e.name).join(",")}`}
          busy={busy}
          activeProject={browser.activeProject}
          projects={projects.projects}
          globalAssetRoot={gitSync.assetRoot}
          selectedCount={browser.selectedEntries.length}
          selectedNames={browser.selectedEntries.map((e) => e.name)}
          selectedKinds={[...new Set(browser.selectedEntries.map((e) => e.kind))]}
          onCancel={ops.closeCopyToProject}
          onSubmit={(toProject) => void ops.submitCopyToProject(toProject)}
        />
      ) : null}

      {ops.addSkillOpen ? (
        <AddSkillModal
          busy={busy}
          platformLabel={PLATFORM_NAME[browser.activePlatform]}
          onCancel={ops.closeAddSkill}
          onSubmit={(source, skillName) =>
            void ops.submitAddSkill(source, skillName)
          }
        />
      ) : null}

      {ops.addMcpOpen ? (
        <AddMcpModal
          busy={busy}
          onCancel={ops.closeAddMcp}
          onSubmit={(name, configJson) =>
            void ops.submitAddMcp(name, configJson)
          }
        />
      ) : null}

      {ops.marketplaceOpen ? (
        <MarketplaceSkillModal
          busy={busy}
          activePlatform={browser.activePlatform}
          activeProject={browser.activeProject}
          setBusy={setBusy}
          onCancel={ops.closeAddMarketplace}
          onImported={(msg) => void ops.handleMarketplaceImported(msg)}
          onError={ops.handleMarketplaceError}
        />
      ) : null}
    </div>
  );
}
