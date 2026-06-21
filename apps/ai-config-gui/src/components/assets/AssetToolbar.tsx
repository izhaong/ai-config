import { FolderOpen } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { assetKindLabel } from "../../i18n/labels";
import { PLATFORM_NAME } from "../../platformIcons";
import type { AssetKind, DeployPlatform, Platform, PlatformAssetEntry } from "../../types";
import { aggregateLinkStateForUi } from "../../utils/aggregatePlatformState";
import { canUpdateEntry } from "../../utils/entryUpdate";
import { RowSyncActions } from "./RowSyncActions";

interface AssetToolbarProps {
  activeKind: AssetKind;
  activePlatform: Platform;
  activeProject: string;
  browsingSource: boolean;
  allVisibleSelected: boolean;
  visibleCount: number;
  selectedCount: number;
  selectedEntries: PlatformAssetEntry[];
  browsePath: string;
  loading: boolean;
  busy: boolean;
  issueReasonForKind: (plat: DeployPlatform) => string | undefined;
  onToggleSelectAll: () => void;
  onBatchSyncPlatform: (plat: Platform) => void;
  onBatchUpdate: () => void;
  onBatchDelete: () => void;
  onOpenFolder: () => void;
  onAddSkill?: () => void;
  onAddMcp?: () => void;
  onAddMarketplace?: () => void;
}

export function AssetToolbar({
  activeKind,
  activePlatform,
  activeProject,
  browsingSource,
  allVisibleSelected,
  visibleCount,
  selectedCount,
  selectedEntries,
  browsePath,
  loading,
  busy,
  issueReasonForKind,
  onToggleSelectAll,
  onBatchSyncPlatform,
  onBatchUpdate,
  onBatchDelete,
  onOpenFolder,
  onAddSkill,
  onAddMcp,
  onAddMarketplace,
}: AssetToolbarProps) {
  const { t } = useTranslation();
  const kindLabel = assetKindLabel(t, activeKind);
  const toolbarTitle = `${kindLabel} · ${PLATFORM_NAME[activePlatform]} · ${
    activeProject === "user-global" ? t("nav.userGlobal") : activeProject
  }`;

  const batchLinkStateFor = useMemo(
    () => (plat: Platform) => aggregateLinkStateForUi(selectedEntries, plat),
    [selectedEntries],
  );

  const batchDeleteDisabled =
    loading || busy || selectedCount === 0 || visibleCount === 0;

  const batchUpdateDisabled =
    loading ||
    busy ||
    selectedCount === 0 ||
    visibleCount === 0 ||
    !selectedEntries.some((entry) =>
      canUpdateEntry(entry, (plat) => !!issueReasonForKind(plat)),
    );

  return (
    <header className="list-header">
      <div className="list-row-shell list-row-shell-header">
        <label className="list-col-check" title={t("toolbar.selectAllTitle")}>
          <input
            type="checkbox"
            checked={allVisibleSelected && visibleCount > 0}
            onChange={onToggleSelectAll}
            disabled={visibleCount === 0 || loading}
          />
        </label>

        <div className="list-col-main list-header-main">
          <div className="list-header-line list-header-line-primary">
            <h3 className="list-header-title" title={toolbarTitle}>
              {toolbarTitle}
            </h3>
            <span className="list-header-meta">
              {t("toolbar.items", { count: visibleCount })}
              {selectedCount > 0
                ? t("toolbar.selected", { count: selectedCount })
                : ""}
              {loading || busy ? t("toolbar.processing") : ""}
            </span>
          </div>
          <div className="list-header-line list-header-line-sub">
            <button
              type="button"
              className="toolbar-icon-btn toolbar-icon-btn-sm"
              disabled={!browsePath || loading || busy}
              title={t("toolbar.openFolderTitle")}
              aria-label={t("toolbar.openFolder")}
              onClick={onOpenFolder}
            >
              <FolderOpen size={12} />
            </button>
            {browsePath ? (
              <code className="list-header-path" title={browsePath}>
                {browsePath}
              </code>
            ) : (
              <span className="list-header-path list-header-path-empty" />
            )}
          </div>
        </div>

        <div className="list-col-actions">
          <RowSyncActions
            variant="batch"
            loading={loading || busy}
            activePlatform={activePlatform}
            browsingSource={browsingSource}
            selectedCount={selectedCount}
            issueReasonFor={issueReasonForKind}
            linkStateFor={batchLinkStateFor}
            onPlatformClick={onBatchSyncPlatform}
            onAdd={
              activeKind === "skill"
                ? onAddSkill
                : activeKind === "mcp" && browsingSource
                  ? onAddMcp
                  : undefined
            }
            addDisabled={loading || busy}
            addTitle={
              activeKind === "mcp"
                ? t("toolbar.addMcpTitle")
                : t("toolbar.addSkillTitle")
            }
            onAddMarketplace={activeKind === "skill" ? onAddMarketplace : undefined}
            addMarketplaceTitle={t("toolbar.addMarketplaceTitle")}
            onUpdate={onBatchUpdate}
            updateDisabled={batchUpdateDisabled}
            updateTitle={t("toolbar.batchUpdateTitle")}
            onDelete={onBatchDelete}
            deleteDisabled={batchDeleteDisabled}
            deleteTitle={t("toolbar.batchDeleteTitle")}
          />
        </div>
      </div>
    </header>
  );
}
