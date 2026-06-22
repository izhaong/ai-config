import { useTranslation } from "react-i18next";

import { AssetDrawer } from "./AssetDrawer";
import { AssetRow } from "./AssetRow";
import { assetKindLabel } from "../../i18n/labels";
import { PLATFORM_NAME } from "../../platformIcons";
import { entryKey } from "../../utils/entryKey";
import type { RowSyncLeadSlot } from "../../utils/rowSyncLeadSlot";
import type { AssetDetail, AssetKind, DeployPlatform, Platform, PlatformAssetEntry } from "../../types";

interface AssetListPaneProps {
  platformList: { entries: PlatformAssetEntry[] } | null;
  visible: PlatformAssetEntry[];
  activeKind: AssetKind;
  activePlatform: Platform;
  leadSlot: RowSyncLeadSlot;
  platformKindUnsupported: boolean;
  loading: boolean;
  busy: boolean;
  selectedKeys: Set<string>;
  drawerName: string | null;
  drawerDetail: AssetDetail | null;
  drawerLoading: boolean;
  drawerEditing: boolean;
  drawerDraft: string;
  drawerReadOnly: boolean;
  onToggleSelect: (key: string, checked: boolean) => void;
  onOpenAsset: (entry: PlatformAssetEntry) => void;
  onPlatformToggle: (entry: PlatformAssetEntry, plat: Platform) => void;
  onUpdateEntry: (entry: PlatformAssetEntry) => void;
  onDeleteEntry: (entry: PlatformAssetEntry) => void;
  onCloseDrawer: () => void;
  onDraftChange: (value: string) => void;
  onEdit: () => void;
  onCancelEdit: () => void;
  onSave: () => void;
  onDelete: () => void;
  issueReasonFor: (entry: PlatformAssetEntry, plat: DeployPlatform) => string | undefined;
}

export function AssetListPane({
  platformList,
  visible,
  activeKind,
  activePlatform,
  leadSlot,
  platformKindUnsupported,
  loading,
  busy,
  selectedKeys,
  drawerName,
  drawerDetail,
  drawerLoading,
  drawerEditing,
  drawerDraft,
  drawerReadOnly,
  onToggleSelect,
  onOpenAsset,
  onPlatformToggle,
  onUpdateEntry,
  onDeleteEntry,
  onCloseDrawer,
  onDraftChange,
  onEdit,
  onCancelEdit,
  onSave,
  onDelete,
  issueReasonFor,
}: AssetListPaneProps) {
  const { t } = useTranslation();
  const kindLabel = assetKindLabel(t, activeKind);
  const rowLoading = loading || busy;
  const showInitialLoading = platformList === null && loading;

  return (
    <div className="content-pane-body">
      {showInitialLoading ? (
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
            return (
              <AssetRow
                key={key}
                rowKey={key}
                entry={entry}
                loading={rowLoading}
                activePlatform={activePlatform}
                leadSlot={leadSlot}
                checked={selectedKeys.has(key)}
                onCheckedChange={onToggleSelect}
                selected={drawerName === entry.name}
                onOpenAsset={onOpenAsset}
                issueReasonFor={issueReasonFor}
                onPlatformToggle={onPlatformToggle}
                onUpdateEntry={onUpdateEntry}
                onDeleteEntry={onDeleteEntry}
              />
            );
          })}
        </div>
      )}

      <AssetDrawer
        open={!!drawerName}
        kind={activeKind}
        name={drawerName ?? ""}
        detail={drawerDetail}
        loading={drawerLoading || busy}
        editing={drawerEditing}
        draft={drawerDraft}
        readOnly={drawerReadOnly}
        onClose={onCloseDrawer}
        onDraftChange={onDraftChange}
        onEdit={onEdit}
        onCancelEdit={onCancelEdit}
        onSave={onSave}
        onDelete={onDelete}
      />
    </div>
  );
}
