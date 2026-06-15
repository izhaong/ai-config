import { useTranslation } from "react-i18next";

import { platformUiLabel } from "./i18n/labels";
import type { DeployPlatform, Platform, PlatformAssetEntry } from "./types";
import { ALL_PLATFORMS, isPlatformActive } from "./types";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "./platformIcons";

interface AssetRowProps {
  entry: PlatformAssetEntry;
  loading: boolean;
  issueReasonFor?: (plat: DeployPlatform) => string | undefined;
  selected?: boolean;
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  onOpen?: () => void;
  onPlatformToggle: (plat: Platform) => void;
}

export function AssetRow({
  entry,
  loading,
  issueReasonFor,
  selected,
  checked = false,
  onCheckedChange,
  onOpen,
  onPlatformToggle,
}: AssetRowProps) {
  const { t } = useTranslation();

  return (
    <div className={`asset-row${selected ? " selected" : ""}${checked ? " checked" : ""}`}>
      {onCheckedChange ? (
        <label className="asset-row-check" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={checked}
            onChange={(e) => onCheckedChange(e.target.checked)}
          />
        </label>
      ) : null}
      <div
        className={`asset-row-main${onOpen ? " clickable" : ""}`}
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
        <div className="desc">{entry.description || t("drawer.noDescription")}</div>
      </div>

      <div className="platform-actions">
        {ALL_PLATFORMS.map((p) => {
          const state = entry.states[p];
          const active = isPlatformActive(state);
          const isDeploy = p !== "aiconfig";
          const issueReason = isDeploy ? issueReasonFor?.(p) : undefined;
          const platUnsupported = !!issueReason;
          const disabled = loading || platUnsupported;
          const title = platUnsupported
            ? t("platform.unsupported", {
                platform: PLATFORM_NAME[p],
                reason: issueReason,
              })
            : p === "aiconfig"
              ? `${PLATFORM_NAME[p]} · ${platformUiLabel(t, active)} · ${
                  active ? t("platform.clickRemoveSource") : t("platform.clickImportSource")
                }`
              : `${PLATFORM_NAME[p]} · ${platformUiLabel(t, active)} · ${
                  active ? t("platform.clickRemove") : t("platform.clickDeploy")
                }`;

          return (
            <button
              key={p}
              type="button"
              className={`plat-btn ${active ? "active" : "inactive"}${platUnsupported ? " unsupported" : ""}`}
              disabled={disabled}
              title={title}
              onClick={() => onPlatformToggle(p)}
            >
              <img src={PLATFORM_FAVICON[p]} alt={PLATFORM_NAME[p]} draggable={false} />
            </button>
          );
        })}
      </div>
    </div>
  );
}
