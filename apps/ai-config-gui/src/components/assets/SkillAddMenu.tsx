import { useMemoizedFn } from "ahooks";
import { MoreHorizontal, Plus, ShoppingBag } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

import { PlatButton, PlatButtonSpacer } from "./PlatButton";

interface SkillAddMenuProps {
  loading: boolean;
  disabled?: boolean;
  addTitle?: string;
  addMarketplaceTitle?: string;
  onAdd: () => void;
  onAddMarketplace: () => void;
}

/** Skill 操作列首格「⋯」（28px，与平台 icon 同列） */
export function SkillAddMenu({
  loading,
  disabled = false,
  addTitle,
  addMarketplaceTitle,
  onAdd,
  onAddMarketplace,
}: SkillAddMenuProps) {
  const { t } = useTranslation();

  const pickAdd = useMemoizedFn(() => {
    if (loading || disabled) return;
    onAdd();
  });

  const pickMarketplace = useMemoizedFn(() => {
    if (loading || disabled) return;
    onAddMarketplace();
  });

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <PlatButton
            state="inactive"
            disabled={loading || disabled}
            className="row-add-menu-btn text-[var(--fg-dim)]"
            aria-label={t("toolbar.addMenuTitle")}
            title={t("toolbar.addMenuTitle")}
            onClick={(e) => e.stopPropagation()}
          />
        }
      >
        <MoreHorizontal size={14} aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="min-w-[220px] font-[family-name:var(--font)] text-[13px]"
        onClick={(e) => e.stopPropagation()}
      >
        <DropdownMenuItem onClick={pickAdd} title={addTitle}>
          <Plus size={14} aria-hidden />
          <span className="min-w-0 flex-1 truncate">{addTitle}</span>
        </DropdownMenuItem>
        <DropdownMenuItem onClick={pickMarketplace} title={addMarketplaceTitle}>
          <ShoppingBag size={14} aria-hidden />
          <span className="min-w-0 flex-1 truncate">{addMarketplaceTitle}</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function LeadSlotSpacer() {
  return <PlatButtonSpacer />;
}

interface RowIconButtonProps {
  className?: string;
  disabled?: boolean;
  title?: string;
  "aria-label"?: string;
  onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void;
  onBlur?: () => void;
  children: React.ReactNode;
}

/** 行内更新/删除等 28px 图标按钮 */
export function RowIconButton({
  className,
  disabled,
  title,
  "aria-label": ariaLabel,
  onClick,
  onBlur,
  children,
}: RowIconButtonProps) {
  return (
    <Button
      type="button"
      variant="outline"
      size="icon-sm"
      disabled={disabled}
      title={title}
      aria-label={ariaLabel}
      onClick={onClick}
      onBlur={onBlur}
      className={cn(
        "plat-btn row-update-btn size-7 min-h-7 min-w-7 rounded-md border-2 border-[var(--border)] bg-[var(--bg-elev-2)] p-0 text-[var(--fg-dim)] shadow-none",
        className,
      )}
    >
      {children}
    </Button>
  );
}
