import { useTranslation } from "react-i18next";

import type { DeployPlatform, Platform, PlatformAssetEntry } from "../../types";
import { canUpdateEntry } from "../../utils/entryUpdate";
import { RowSyncActions } from "./RowSyncActions";
import { sanitizeListDescription } from "../../utils/sanitizeListDescription";

interface AssetRowProps {
  entry: PlatformAssetEntry;
  loading: boolean;
  activePlatform: Platform;
  issueReasonFor?: (plat: DeployPlatform) => string | undefined;
  selected?: boolean;
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  onOpen?: () => void;
  onPlatformToggle: (plat: Platform) => void;
  onUpdate: () => void;
  onDelete: () => void;
}

export function AssetRow({
  entry,
  loading,
  activePlatform,
  issueReasonFor,
  selected,
  checked = false,
  onCheckedChange,
  onOpen,
  onPlatformToggle,
  onUpdate,
  onDelete,
}: AssetRowProps) {
  const { t } = useTranslation();

  return (
    <div className={`asset-row list-row-shell${selected ? " selected" : ""}${checked ? " checked" : ""}`}>
      {onCheckedChange ? (
        <label className="list-col-check" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={checked}
            onChange={(e) => onCheckedChange(e.target.checked)}
          />
        </label>
      ) : null}
      <div
        className={`list-col-main asset-row-main${onOpen ? " clickable" : ""}`}
        onClick={onOpen}
        onKeyDown={
          onOpen
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpen();
                }
              }
            : undefined
        }
        role={onOpen ? "button" : undefined}
        tabIndex={onOpen ? 0 : undefined}
      >
        <div className="name">{entry.name}</div>
        <div className="desc">
          {sanitizeListDescription(entry.description) ||
            t("drawer.noDescription")}
        </div>
      </div>

      <div className="list-col-actions">
        <RowSyncActions
          loading={loading}
          activePlatform={activePlatform}
          issueReasonFor={issueReasonFor}
          linkStateFor={(plat) => entry.states[plat]}
          onPlatformClick={onPlatformToggle}
          onUpdate={onUpdate}
          updateDisabled={
            !canUpdateEntry(entry, (plat) => !!issueReasonFor?.(plat))
          }
          updateTitle={t("toolbar.rowUpdateTitle", { name: entry.name })}
          onDelete={onDelete}
          deleteTitle={t("toolbar.rowDeleteTitle", { name: entry.name })}
        />
      </div>
    </div>
  );
}
