import { useMemoizedFn, useTimeout } from "ahooks";
import { Plus, RefreshCw, ShoppingBag, Trash2 } from "lucide-react";
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
  activePlatform?: Platform;
  selectedCount?: number;
  issueReasonFor?: (plat: DeployPlatform) => string | undefined;
  linkStateFor?: (plat: Platform) => LinkState | "mixed";
  onPlatformClick: (plat: Platform) => void;
  /** 从远程仓库添加 skill（仅 batch 工具栏） */
  onAdd?: () => void;
  addDisabled?: boolean;
  addTitle?: string;
  onAddMarketplace?: () => void;
  addMarketplaceTitle?: string;
  /** 从 ai-config 源重新下发到已激活平台（复用 deploy） */
  onUpdate?: () => void;
  updateDisabled?: boolean;
  updateTitle: string;
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

  const handleAddMarketplaceClick = useMemoizedFn((e: MouseEvent) => {
    e.stopPropagation();
    if (loading || addDisabled || !onAddMarketplace) return;
    onAddMarketplace();
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

  return (
    <div className="row-sync-actions">
      {variant === "batch" && onAdd ? (
        <button
          type="button"
          className="plat-btn row-add-btn"
          disabled={loading || addDisabled}
          title={addTitle}
          aria-label={addTitle}
          onClick={handleAddClick}
        >
          <Plus size={14} aria-hidden />
        </button>
      ) : null}
      {variant === "batch" && onAddMarketplace ? (
        <button
          type="button"
          className="plat-btn row-add-marketplace-btn"
          disabled={loading || addDisabled}
          title={addMarketplaceTitle}
          aria-label={addMarketplaceTitle}
          onClick={handleAddMarketplaceClick}
        >
          <ShoppingBag size={14} aria-hidden />
        </button>
      ) : null}
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
      <button
        type="button"
        className="plat-btn row-update-btn"
        disabled={loading || updateDisabled || !onUpdate}
        title={updateTitle}
        aria-label={updateTitle}
        onClick={handleUpdateClick}
      >
        <RefreshCw size={14} aria-hidden />
      </button>
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
