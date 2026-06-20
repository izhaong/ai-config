import { useTranslation } from "react-i18next";

import { AssetDrawer } from "./AssetDrawer";
import { AssetRow } from "./AssetRow";
import { assetKindLabel } from "../../i18n/labels";
import { PLATFORM_NAME } from "../../platformIcons";
import { entryKey } from "../../utils/entryKey";
import type { AssetDetail, AssetKind, DeployPlatform, Platform, PlatformAssetEntry } from "../../types";

interface AssetListPaneProps {
  platformList: { entries: PlatformAssetEntry[] } | null;
  visible: PlatformAssetEntry[];
  activeKind: AssetKind;
  activePlatform: Platform;
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
  canOpenEntry: (entry: PlatformAssetEntry) => boolean;
  issueReasonFor: (entry: PlatformAssetEntry, plat: DeployPlatform) => string | undefined;
  onToggleSelect: (key: string, checked: boolean) => void;
  onOpenAsset: (entry: PlatformAssetEntry) => void;
  onPlatformToggle: (entry: PlatformAssetEntry, plat: Platform) => void;
  onDeleteEntry: (entry: PlatformAssetEntry) => void;
  onCloseDrawer: () => void;
  onDraftChange: (value: string) => void;
  onEdit: () => void;
  onCancelEdit: () => void;
  onSave: () => void;
  onDelete: () => void;
}

export function AssetListPane({
  platformList,
  visible,
  activeKind,
  activePlatform,
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
  canOpenEntry,
  issueReasonFor,
  onToggleSelect,
  onOpenAsset,
  onPlatformToggle,
  onDeleteEntry,
  onCloseDrawer,
  onDraftChange,
  onEdit,
  onCancelEdit,
  onSave,
  onDelete,
}: AssetListPaneProps) {
  const { t } = useTranslation();
  const kindLabel = assetKindLabel(t, activeKind);

  return (
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
            const canOpen = canOpenEntry(entry);
            return (
              <AssetRow
                key={key}
                entry={entry}
                loading={loading || busy}
                checked={selectedKeys.has(key)}
                onCheckedChange={(checked) => onToggleSelect(key, checked)}
                selected={drawerName === entry.name}
                onOpen={canOpen ? () => onOpenAsset(entry) : undefined}
                issueReasonFor={(plat) => issueReasonFor(entry, plat)}
                onPlatformToggle={(plat) => onPlatformToggle(entry, plat)}
                onDelete={() => onDeleteEntry(entry)}
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
          readOnly={drawerReadOnly}
          onClose={onCloseDrawer}
          onDraftChange={onDraftChange}
          onEdit={onEdit}
          onCancelEdit={onCancelEdit}
          onSave={onSave}
          onDelete={onDelete}
        />
      ) : null}
    </div>
  );
}
