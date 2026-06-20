import { useMemoizedFn, useTimeout } from "ahooks";
import { Trash2 } from "lucide-react";
import { useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";

import type { DeployPlatform, LinkState, Platform } from "../../types";
import { PlatformIconButtons } from "./PlatformIconButtons";

const DELETE_ARM_MS = 4000;

interface RowSyncActionsProps {
  loading: boolean;
  disabled?: boolean;
  variant?: "row" | "batch";
  browsingSource?: boolean;
  selectedCount?: number;
  issueReasonFor?: (plat: DeployPlatform) => string | undefined;
  linkStateFor?: (plat: Platform) => LinkState | "mixed";
  onPlatformClick: (plat: Platform) => void;
  /** 二次确认后由父级弹出 ConfirmModal 并执行删除 */
  onDelete?: () => void;
  deleteDisabled?: boolean;
  deleteTitle: string;
}

export function RowSyncActions({
  loading,
  disabled = false,
  variant = "row",
  browsingSource = false,
  selectedCount = 0,
  issueReasonFor,
  linkStateFor,
  onPlatformClick,
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

  return (
    <div className="row-sync-actions">
      <PlatformIconButtons
        loading={loading}
        disabled={disabled}
        variant={variant}
        browsingSource={browsingSource}
        selectedCount={selectedCount}
        issueReasonFor={issueReasonFor}
        linkStateFor={linkStateFor}
        onPlatformClick={onPlatformClick}
      />
      <button
        type="button"
        className={`plat-btn row-delete-btn${deleteArmed ? " row-delete-btn-armed" : ""}`}
        disabled={loading || deleteDisabled}
        title={deleteButtonTitle}
        aria-label={deleteButtonTitle}
        onClick={handleDeleteClick}
        onBlur={resetDeleteArm}
      >
        <Trash2 size={14} aria-hidden />
      </button>
    </div>
  );
}
