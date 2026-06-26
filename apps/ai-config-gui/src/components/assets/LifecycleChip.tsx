import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes } from "react";

import { cn } from "@/lib/utils";

const lifecycleChipVariants = cva(
  [
    "hook-lifecycle-chip inline-flex h-6 max-w-full shrink-0 items-center",
    "rounded-full border px-2.5 text-[10px] font-medium leading-none",
    "transition-[border-color,background-color,color,opacity] duration-100",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40",
  ].join(" "),
  {
    variants: {
      state: {
        active: [
          "border-[rgba(78,201,168,0.55)] bg-[rgba(78,201,168,0.16)] font-semibold text-[var(--ok)]",
          "hover:border-[rgba(229,116,116,0.55)] hover:bg-[rgba(229,116,116,0.14)] hover:text-[var(--err)]",
        ].join(" "),
        inactive: [
          "border-[var(--border)] bg-[var(--bg-elev-2)] text-[var(--fg-dim)]",
          "hover:border-[rgba(94,155,255,0.45)] hover:bg-[rgba(94,155,255,0.1)] hover:text-[var(--fg)]",
        ].join(" "),
        unsupported: [
          "cursor-not-allowed border-[var(--border)]/50 bg-transparent font-medium text-[var(--fg-dim)] opacity-35",
          "hover:border-[var(--border)]/50 hover:bg-transparent hover:text-[var(--fg-dim)]",
        ].join(" "),
      },
    },
    defaultVariants: {
      state: "inactive",
    },
  },
);

export type LifecycleChipState = NonNullable<
  VariantProps<typeof lifecycleChipVariants>["state"]
>;

type LifecycleChipProps = ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof lifecycleChipVariants>;

export function LifecycleChip({
  className,
  state,
  children,
  ...props
}: LifecycleChipProps) {
  return (
    <button
      type="button"
      data-slot="lifecycle-chip"
      className={cn(lifecycleChipVariants({ state }), className)}
      {...props}
    >
      <span className="truncate">{children}</span>
    </button>
  );
}
