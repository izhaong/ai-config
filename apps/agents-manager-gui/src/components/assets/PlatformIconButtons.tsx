import { useTranslation } from "react-i18next";

import { platformUiLabel } from "../../i18n/labels";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "../../platformIcons";
import type { DeployPlatform, LinkState, Platform } from "../../types";
import { ALL_PLATFORMS, isPlatformActive } from "../../types";
import { PlatButton } from "./PlatButton";

interface PlatformIconButtonsProps {
  loading: boolean;
  disabled?: boolean;
  variant?: "row" | "batch";
  /** 左侧当前浏览的平台；对应 icon 仅展示状态、不可点击 */
  activePlatform?: Platform;
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
  activePlatform,
  browsingSource = false,
  selectedCount = 0,
  issueReasonFor,
  linkStateFor,
  onPlatformClick,
}: PlatformIconButtonsProps) {
  const { t } = useTranslation();
  const isBatch = variant === "batch";

  return (
    <div className="platform-actions ml-auto flex shrink-0 flex-nowrap items-center gap-[5px]">
      {ALL_PLATFORMS.map((p) => {
        const isDeploy = p !== "agentsmanager";
        const issueReason = isDeploy ? issueReasonFor?.(p) : undefined;
        const platUnsupported = !!issueReason;
        const state = linkStateFor?.(p);
        const active =
          state !== "mixed" && state !== undefined && isPlatformActive(state);
        const partial = state === "mixed";
        const batchNoSelection = isBatch && selectedCount === 0;
        const isBrowseCurrent =
          activePlatform !== undefined && p === activePlatform;
        const btnDisabled =
          loading ||
          disabled ||
          platUnsupported ||
          batchNoSelection ||
          isBrowseCurrent;

        const uiLabel = partial
          ? t("platform.partialLinked")
          : platformUiLabel(t, active);

        const batchActionHint =
          isBatch && !active
            ? t("platform.batchClickActivateAll")
            : isBatch && active
              ? t("platform.clickRemove")
              : partial
                ? t("platform.batchClickMixed")
                : active
                  ? t("platform.clickRemove")
                  : p === "agentsmanager"
                    ? t("platform.clickImportSource")
                    : t("platform.clickDeploy");

        const ariaPressed = partial ? "mixed" : active ? "true" : "false";

        const title = platUnsupported
          ? t("platform.unsupported", {
              platform: PLATFORM_NAME[p],
              reason: issueReason,
            })
          : isBrowseCurrent
            ? t("platform.browseCurrentStatus", {
                platform: PLATFORM_NAME[p],
                state: uiLabel,
              })
            : isBatch
              ? batchNoSelection
                ? t("toolbar.batchSyncNeedSelection")
                : p === "agentsmanager"
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
              : p === "agentsmanager"
                ? `${PLATFORM_NAME[p]} · ${uiLabel} · ${
                    active
                      ? t("platform.clickRemove")
                      : t("platform.clickImportSource")
                  }`
                : `${PLATFORM_NAME[p]} · ${uiLabel} · ${
                    active
                      ? t("platform.clickRemove")
                      : t("platform.clickDeploy")
                  }`;

        const platState = partial ? "partial" : active ? "active" : "inactive";

        return (
          <PlatButton
            key={p}
            state={platState}
            batch={isBatch}
            unsupported={platUnsupported}
            browseCurrent={isBrowseCurrent}
            disabled={btnDisabled}
            title={title}
            aria-pressed={ariaPressed}
            aria-disabled={isBrowseCurrent ? true : undefined}
            onClick={() => {
              if (!isBrowseCurrent) onPlatformClick(p);
            }}
          >
            <img src={PLATFORM_FAVICON[p]} alt={PLATFORM_NAME[p]} draggable={false} />
          </PlatButton>
        );
      })}
    </div>
  );
}
