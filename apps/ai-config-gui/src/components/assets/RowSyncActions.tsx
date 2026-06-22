import { useMemoizedFn, useTimeout } from "ahooks";
import { Plus, RefreshCw, Trash2 } from "lucide-react";
import { useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import type { DeployPlatform, LinkState, Platform } from "../../types";
import type { RowSyncLeadSlot } from "../../utils/rowSyncLeadSlot";
import { PlatformIconButtons } from "./PlatformIconButtons";
import { LeadSlotSpacer, RowIconButton, SkillAddMenu } from "./SkillAddMenu";
import { PlatButton } from "./PlatButton";

const DELETE_ARM_MS = 4000;

interface RowSyncActionsProps {
  loading: boolean;
  disabled?: boolean;
  variant?: "row" | "batch";
  /** 操作列首格：skill ⋯ / mcp + / 无 */
  leadSlot?: RowSyncLeadSlot;
  browsingSource?: boolean;
  activePlatform?: Platform;
  selectedCount?: number;
  issueReasonFor?: (plat: DeployPlatform) => string | undefined;
  linkStateFor?: (plat: Platform) => LinkState | "mixed";
  onPlatformClick: (plat: Platform) => void;
  onAdd?: () => void;
  addDisabled?: boolean;
  addTitle?: string;
  onAddMarketplace?: () => void;
  addMarketplaceTitle?: string;
  onUpdate?: () => void;
  updateDisabled?: boolean;
  updateTitle: string;
  onDelete?: () => void;
  deleteDisabled?: boolean;
  deleteTitle: string;
}

/** 顶栏 batch 与列表行共用同一 DOM 结构 */
export function RowSyncActions({
  loading,
  disabled = false,
  variant = "row",
  leadSlot = "none",
  browsingSource = false,
  activePlatform,
  selectedCount = 0,
  issueReasonFor,
  linkStateFor,
  onPlatformClick,
  onAdd,
  addDisabled = false,
  addTitle,
  onAddMarketplace,
  addMarketplaceTitle,
  onUpdate,
  updateDisabled = false,
  updateTitle,
  onDelete,
  deleteDisabled = false,
  deleteTitle,
}: RowSyncActionsProps) {
  const { t } = useTranslation();
  const [deleteArmed, setDeleteArmed] = useState(false);

  useTimeout(
    () => setDeleteArmed(false),
    deleteArmed ? DELETE_ARM_MS : undefined,
  );

  const resetDeleteArm = useMemoizedFn(() => setDeleteArmed(false));

  const handleAddClick = useMemoizedFn((e: MouseEvent) => {
    e.stopPropagation();
    if (loading || addDisabled || !onAdd) return;
    onAdd();
  });

  const handleUpdateClick = useMemoizedFn((e: MouseEvent) => {
    e.stopPropagation();
    if (loading || updateDisabled || !onUpdate) return;
    onUpdate();
  });

  const handleDeleteClick = useMemoizedFn((e: MouseEvent) => {
    e.stopPropagation();
    if (loading || deleteDisabled || !onDelete) return;

    if (!deleteArmed) {
      setDeleteArmed(true);
      return;
    }

    resetDeleteArm();
    onDelete();
  });

  const deleteButtonTitle = deleteArmed
    ? t("toolbar.deleteConfirmHint")
    : deleteTitle;

  const showLead = leadSlot !== "none";

  const leadCell =
    leadSlot === "skill-menu" && variant === "batch" && onAdd && onAddMarketplace ? (
      <SkillAddMenu
        loading={loading}
        disabled={addDisabled}
        addTitle={addTitle}
        addMarketplaceTitle={addMarketplaceTitle}
        onAdd={onAdd}
        onAddMarketplace={onAddMarketplace}
      />
    ) : leadSlot === "mcp-add" && variant === "batch" && onAdd ? (
      <PlatButton
        state="inactive"
        disabled={loading || addDisabled}
        className="row-add-btn text-[var(--fg-dim)]"
        title={addTitle}
        aria-label={addTitle}
        onClick={handleAddClick}
      >
        <Plus size={14} aria-hidden />
      </PlatButton>
    ) : showLead ? (
      <LeadSlotSpacer />
    ) : null;

  return (
    <div className="row-sync-actions flex w-full shrink-0 items-center justify-end gap-[5px]">
      {leadCell}
      <PlatformIconButtons
        loading={loading}
        disabled={disabled}
        variant={variant}
        activePlatform={activePlatform}
        browsingSource={browsingSource}
        selectedCount={selectedCount}
        issueReasonFor={issueReasonFor}
        linkStateFor={linkStateFor}
        onPlatformClick={onPlatformClick}
      />
      <RowIconButton
        className="row-update-btn hover:border-[rgba(94,155,255,0.45)] hover:bg-[rgba(94,155,255,0.1)] hover:text-[var(--accent)] disabled:opacity-[0.32]"
        disabled={loading || updateDisabled || !onUpdate}
        title={updateTitle}
        aria-label={updateTitle}
        onClick={handleUpdateClick}
      >
        <RefreshCw size={14} aria-hidden />
      </RowIconButton>
      <RowIconButton
        className={cn(
          "row-delete-btn hover:border-[rgba(229,116,116,0.45)] hover:bg-[rgba(229,116,116,0.1)] hover:text-[var(--err)] hover:opacity-100 disabled:opacity-[0.32]",
          deleteArmed &&
            "row-delete-btn-armed border-[rgba(229,116,116,0.65)] bg-[rgba(229,116,116,0.14)] text-[var(--err)] opacity-100 animate-[row-delete-arm-pulse_1.2s_ease-in-out_infinite]",
        )}
        disabled={loading || deleteDisabled}
        title={deleteButtonTitle}
        aria-label={deleteButtonTitle}
        onClick={handleDeleteClick}
        onBlur={resetDeleteArm}
      >
        <Trash2 size={14} aria-hidden />
      </RowIconButton>
    </div>
  );
}
