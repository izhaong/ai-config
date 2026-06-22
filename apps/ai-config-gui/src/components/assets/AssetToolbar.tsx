import { FolderOpen } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { assetKindLabel } from "../../i18n/labels";
import { PLATFORM_NAME } from "../../platformIcons";
import type { AssetKind, DeployPlatform, Platform, PlatformAssetEntry } from "../../types";
import { aggregateLinkStateForUi } from "../../utils/aggregatePlatformState";
import { canUpdateEntry } from "../../utils/entryUpdate";
import { rowSyncLeadSlot } from "../../utils/rowSyncLeadSlot";
import {
  ListColActions,
  ListColCheck,
  ListColMain,
  ListRowShell,
} from "./ListRowShell";
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
  const leadSlot = rowSyncLeadSlot(activeKind, browsingSource);

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

  const selectAllChecked = allVisibleSelected && visibleCount > 0;

  return (
    <header className="list-header">
      <ListRowShell variant="header" className="list-row-shell-header">
        <ListColCheck title={t("toolbar.selectAllTitle")}>
          <Checkbox
            checked={selectAllChecked}
            onCheckedChange={() => onToggleSelectAll()}
            disabled={visibleCount === 0 || loading}
            aria-label={t("toolbar.selectAllTitle")}
          />
        </ListColCheck>

        <ListColMain className="list-header-main flex min-w-0 flex-col gap-px">
          <div className="list-header-line list-header-line-primary flex min-w-0 items-center gap-1.5">
            <h3
              className="list-header-title m-0 min-w-0 flex-1 truncate text-[13px] font-semibold leading-snug"
              title={toolbarTitle}
            >
              {toolbarTitle}
            </h3>
            <span className="list-header-meta shrink-0 text-[11px] text-[var(--fg-dim)]">
              {t("toolbar.items", { count: visibleCount })}
              {selectedCount > 0
                ? t("toolbar.selected", { count: selectedCount })
                : ""}
              {loading || busy ? t("toolbar.processing") : ""}
            </span>
          </div>
          <div className="list-header-line list-header-line-sub flex min-w-0 items-center gap-1.5">
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              disabled={!browsePath || loading || busy}
              title={t("toolbar.openFolderTitle")}
              aria-label={t("toolbar.openFolder")}
              onClick={onOpenFolder}
              className="toolbar-icon-btn toolbar-icon-btn-sm size-6 shrink-0 rounded border border-[var(--border)] bg-[var(--bg-elev)] p-0 text-[var(--fg-dim)] hover:bg-[var(--bg-elev-2)]"
            >
              <FolderOpen size={12} />
            </Button>
            {browsePath ? (
              <code
                className="list-header-path min-w-0 flex-1 truncate font-[family-name:var(--font)] text-[11px] text-[var(--fg-dim)]"
                title={browsePath}
              >
                {browsePath}
              </code>
            ) : (
              <span className="list-header-path-empty min-h-[1em] flex-1" />
            )}
          </div>
        </ListColMain>

        <ListColActions>
          <RowSyncActions
            variant="batch"
            leadSlot={leadSlot}
            loading={loading || busy}
            activePlatform={activePlatform}
            browsingSource={browsingSource}
            selectedCount={selectedCount}
            issueReasonFor={issueReasonForKind}
            linkStateFor={batchLinkStateFor}
            onPlatformClick={onBatchSyncPlatform}
            onAdd={
              leadSlot === "skill-menu"
                ? onAddSkill
                : leadSlot === "mcp-add"
                  ? onAddMcp
                  : undefined
            }
            addDisabled={loading || busy}
            addTitle={
              activeKind === "mcp"
                ? t("toolbar.addMcpTitle")
                : t("toolbar.addSkillTitle")
            }
            onAddMarketplace={
              leadSlot === "skill-menu" ? onAddMarketplace : undefined
            }
            addMarketplaceTitle={t("toolbar.addMarketplaceTitle")}
            onUpdate={onBatchUpdate}
            updateDisabled={batchUpdateDisabled}
            updateTitle={t("toolbar.batchUpdateTitle")}
            onDelete={onBatchDelete}
            deleteDisabled={batchDeleteDisabled}
            deleteTitle={t("toolbar.batchDeleteTitle")}
          />
        </ListColActions>
      </ListRowShell>
    </header>
  );
}
