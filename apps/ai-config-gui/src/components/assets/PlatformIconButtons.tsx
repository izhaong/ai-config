import { useTranslation } from "react-i18next";

import { platformUiLabel } from "../../i18n/labels";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "../../platformIcons";
import type { DeployPlatform, LinkState, Platform } from "../../types";
import { ALL_PLATFORMS, isPlatformActive } from "../../types";

interface PlatformIconButtonsProps {
  loading: boolean;
  disabled?: boolean;
  variant?: "row" | "batch";
  browsingSource?: boolean;
  selectedCount?: number;
  issueReasonFor?: (plat: DeployPlatform) => string | undefined;
  linkStateFor?: (plat: Platform) => LinkState | "mixed";
  onPlatformClick: (plat: Platform) => void;
}

export function PlatformIconButtons({
  loading,
  disabled = false,
  variant = "row",
  browsingSource = false,
  selectedCount = 0,
  issueReasonFor,
  linkStateFor,
  onPlatformClick,
}: PlatformIconButtonsProps) {
  const { t } = useTranslation();
  const isBatch = variant === "batch";

  return (
    <div className="platform-actions">
      {ALL_PLATFORMS.map((p) => {
        const isDeploy = p !== "aiconfig";
        const issueReason = isDeploy ? issueReasonFor?.(p) : undefined;
        const platUnsupported = !!issueReason;
        const state = linkStateFor?.(p);
        const active =
          state !== "mixed" && state !== undefined && isPlatformActive(state);
        const partial = state === "mixed";
        const batchSourceView = isBatch && browsingSource && p === "aiconfig";
        const batchNoSelection = isBatch && selectedCount === 0;
        const batchAiconfigAllInSource =
          isBatch && p === "aiconfig" && active && !partial;
        const btnDisabled =
          loading ||
          disabled ||
          platUnsupported ||
          batchSourceView ||
          batchNoSelection ||
          batchAiconfigAllInSource;

        const uiLabel = partial
          ? t("platform.partialLinked")
          : platformUiLabel(t, active);

        const batchActionHint =
          isBatch && batchAiconfigAllInSource
            ? t("platform.batchAiconfigUseDeleteButton")
            : isBatch && !active
              ? t("platform.batchClickActivateAll")
              : isBatch && active
                ? t("platform.clickRemove")
                : partial
                  ? t("platform.batchClickMixed")
                  : active
                    ? p === "aiconfig"
                      ? t("platform.aiconfigDeleteNeedsConfirm")
                      : t("platform.clickRemove")
                    : p === "aiconfig"
                      ? t("platform.clickImportSource")
                      : t("platform.clickDeploy");

        const ariaPressed = partial ? "mixed" : active ? "true" : "false";

        const title = platUnsupported
          ? t("platform.unsupported", {
              platform: PLATFORM_NAME[p],
              reason: issueReason,
            })
          : isBatch
            ? batchNoSelection
              ? t("toolbar.batchSyncNeedSelection")
              : p === "aiconfig"
                ? browsingSource
                  ? t("toolbar.batchSyncSourceDisabled")
                  : t("toolbar.batchImportSourceState", {
                      state: uiLabel,
                      hint: batchActionHint,
                    })
                : t("toolbar.batchSyncPlatformState", {
                    platform: PLATFORM_NAME[p],
                    state: uiLabel,
                    hint: batchActionHint,
                  })
            : p === "aiconfig"
              ? `${PLATFORM_NAME[p]} · ${uiLabel} · ${
                  active
                    ? t("platform.aiconfigDeleteNeedsConfirm")
                    : t("platform.clickImportSource")
                }`
              : `${PLATFORM_NAME[p]} · ${uiLabel} · ${
                  active
                    ? t("platform.clickRemove")
                    : t("platform.clickDeploy")
                }`;

        const className = [
          "plat-btn",
          partial ? "partial" : active ? "active" : "inactive",
          isBatch ? "plat-btn-batch" : "",
          platUnsupported ? "unsupported" : "",
        ]
          .filter(Boolean)
          .join(" ");

        return (
          <button
            key={p}
            type="button"
            className={className}
            disabled={btnDisabled}
            title={title}
            aria-pressed={ariaPressed}
            onClick={() => onPlatformClick(p)}
          >
            <img src={PLATFORM_FAVICON[p]} alt={PLATFORM_NAME[p]} draggable={false} />
          </button>
        );
      })}
    </div>
  );
}
