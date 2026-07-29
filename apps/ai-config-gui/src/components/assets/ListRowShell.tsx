import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

interface ListRowShellProps {
  variant?: "header" | "row";
  className?: string;
  children: ReactNode;
}

/** 顶栏与列表行共用三列网格（复选框 | 主内容 | 操作列） */
export function ListRowShell({
  variant = "row",
  className,
  children,
}: ListRowShellProps) {
  return (
    <div
      className={cn(
        "grid w-full min-w-0 box-border items-center",
        "grid-cols-[var(--list-check-w)_minmax(0,1fr)_var(--list-actions-w)]",
        "gap-x-[var(--list-col-gap)]",
        variant === "header" &&
          "min-h-[34px] border border-transparent rounded-md px-[var(--list-row-pad-x)]",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function ListColCheck({
  className,
  children,
  ...props
}: React.ComponentProps<"label">) {
  return (
    <label
      className={cn(
        "list-col-check flex cursor-pointer items-center justify-center",
        className,
      )}
      {...props}
    >
      {children}
    </label>
  );
}

export function ListColMain({
  className,
  children,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div className={cn("min-w-0 overflow-hidden", className)} {...props}>
      {children}
    </div>
  );
}

export function ListColActions({
  className,
  children,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "list-col-actions flex w-[var(--list-actions-w)] min-w-[var(--list-actions-w)] max-w-[var(--list-actions-w)] shrink-0 items-center justify-end",
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}
