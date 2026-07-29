import { memo, useCallback } from "react";
import { useTranslation } from "react-i18next";

import { Checkbox } from "@/components/ui/checkbox";
import { cn } from "@/lib/utils";
import {
  canRemoveFromPlatform,
  isSourcePlatform,
  type DeployPlatform,
  type Platform,
  type PlatformAssetEntry,
} from "../../types";
import type { RowSyncLeadSlot } from "../../utils/rowSyncLeadSlot";
import { canOpenEntry } from "../../utils/canOpenEntry";
import { canUpdateEntry } from "../../utils/entryUpdate";
import { sanitizeListDescription } from "../../utils/sanitizeListDescription";
import {
  ListColActions,
  ListColCheck,
  ListColMain,
  ListRowShell,
} from "./ListRowShell";
import { RowSyncActions } from "./RowSyncActions";
import { HookTypeBadge } from "./HookTypeBadge";

interface AssetRowProps {
  entry: PlatformAssetEntry;
  loading: boolean;
  activePlatform: Platform;
  leadSlot?: RowSyncLeadSlot;
  issueReasonFor?: (entry: PlatformAssetEntry, plat: DeployPlatform) => string | undefined;
  selected?: boolean;
  checked?: boolean;
  onCheckedChange?: (key: string, checked: boolean) => void;
  onOpenAsset?: (entry: PlatformAssetEntry) => void;
  onPlatformToggle: (entry: PlatformAssetEntry, plat: Platform) => void;
  onUpdateEntry: (entry: PlatformAssetEntry) => void;
  onDeleteEntry: (entry: PlatformAssetEntry) => void;
  rowKey: string;
}

function AssetRowInner({
  entry,
  loading,
  activePlatform,
  leadSlot = "none",
  issueReasonFor,
  selected,
  checked = false,
  onCheckedChange,
  onOpenAsset,
  onPlatformToggle,
  onUpdateEntry,
  onDeleteEntry,
  rowKey,
}: AssetRowProps) {
  const { t } = useTranslation();
  const canOpen = !!onOpenAsset && canOpenEntry(entry);
  const isHook = entry.kind === "hook";
  const hookType = entry.hook_type ?? "command";
  const showHookDesc =
    isHook && hookType === "command" && sanitizeListDescription(entry.description);

  const handleOpen = useCallback(() => {
    onOpenAsset?.(entry);
  }, [entry, onOpenAsset]);

  const handlePlatformToggle = useCallback(
    (plat: Platform) => onPlatformToggle(entry, plat),
    [entry, onPlatformToggle],
  );

  const handleUpdate = useCallback(
    () => onUpdateEntry(entry),
    [entry, onUpdateEntry],
  );

  const handleDelete = useCallback(
    () => onDeleteEntry(entry),
    [entry, onDeleteEntry],
  );

  const issueForRow = useCallback(
    (plat: DeployPlatform) => issueReasonFor?.(entry, plat),
    [entry, issueReasonFor],
  );

  return (
    <ListRowShell
      className={cn(
        "asset-row",
        isHook && "asset-row--hook",
        selected && "selected",
        checked && "checked",
      )}
    >
      {onCheckedChange ? (
        <ListColCheck onClick={(e) => e.stopPropagation()}>
          <Checkbox
            checked={checked}
            onCheckedChange={(value) => onCheckedChange(rowKey, value === true)}
          />
        </ListColCheck>
      ) : null}
      <ListColMain
        className={cn("asset-row-main", canOpen && "clickable")}
        onClick={canOpen ? handleOpen : undefined}
        onKeyDown={
          canOpen
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  handleOpen();
                }
              }
            : undefined
        }
        role={canOpen ? "button" : undefined}
        tabIndex={canOpen ? 0 : undefined}
      >
        <div className="name flex min-w-0 items-center gap-2">
          {isHook ? <HookTypeBadge hookType={hookType} /> : null}
          <span className="min-w-0 truncate">{entry.name}</span>
        </div>
        {showHookDesc ? (
          <div className="desc line-clamp-1">{showHookDesc}</div>
        ) : !isHook ? (
          <div className="desc">
            {sanitizeListDescription(entry.description) ||
              t("drawer.noDescription")}
          </div>
        ) : null}
      </ListColMain>

      <ListColActions className={isHook ? "self-start pt-0.5" : undefined}>
        <RowSyncActions
          leadSlot={leadSlot}
          loading={loading}
          activePlatform={activePlatform}
          issueReasonFor={issueForRow}
          linkStateFor={(plat) => entry.states[plat]}
          onPlatformClick={handlePlatformToggle}
          onUpdate={handleUpdate}
          updateDisabled={
            !canUpdateEntry(entry, (plat) => !!issueReasonFor?.(entry, plat))
          }
          updateTitle={t("toolbar.rowUpdateTitle", { name: entry.name })}
          onDelete={handleDelete}
          deleteDisabled={
            !isSourcePlatform(activePlatform) &&
            !canRemoveFromPlatform(entry.kind, entry.states[activePlatform])
          }
          deleteTitle={t("toolbar.rowDeleteTitle", { name: entry.name })}
        />
      </ListColActions>
    </ListRowShell>
  );
}

export const AssetRow = memo(AssetRowInner);
