import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes } from "react";

import { cn } from "@/lib/utils";

const platButtonVariants = cva(
  "plat-btn inline-flex size-7 min-h-7 min-w-7 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded-md border-2 border-[var(--border)] bg-[var(--bg-elev-2)] p-0 transition-[border-color,opacity,box-shadow,filter] duration-100 [&_img]:pointer-events-none [&_img]:size-4 [&_img]:object-contain",
  {
    variants: {
      state: {
        active:
          "border-[var(--ok)] bg-[rgba(78,201,168,0.14)] opacity-100 shadow-[0_0_0_1px_rgba(78,201,168,0.35)] [filter:none] hover:border-[var(--err)] hover:bg-[rgba(229,116,116,0.12)] hover:shadow-[0_0_0_1px_rgba(229,116,116,0.35)]",
        inactive:
          "border-dashed border-[var(--border)] opacity-[0.3] [filter:grayscale(0.85)] hover:border-[var(--accent)] hover:border-solid hover:bg-[rgba(94,155,255,0.1)] hover:opacity-100 hover:[filter:none]",
        partial:
          "border-[var(--accent)] bg-[rgba(94,155,255,0.08)] opacity-[0.88] shadow-[0_0_0_1px_rgba(94,155,255,0.22)] [filter:none] hover:bg-[rgba(94,155,255,0.14)] hover:opacity-100",
      },
      batch: {
        true: "",
        false: "",
      },
      unsupported: {
        true: "cursor-not-allowed opacity-[0.28] [filter:grayscale(1)] hover:opacity-[0.28]",
        false: "",
      },
      browseCurrent: {
        true: "cursor-default hover:shadow-none disabled:cursor-default disabled:opacity-100 disabled:[filter:none]",
        false: "",
      },
    },
    compoundVariants: [
      {
        batch: true,
        state: ["partial", "inactive"],
        className: "border-dashed",
      },
      {
        batch: true,
        state: "partial",
        className: "border-[var(--accent)]",
      },
      {
        batch: true,
        state: "inactive",
        className: "opacity-[0.52] hover:border-[var(--accent)] hover:border-solid hover:bg-[rgba(94,155,255,0.12)] hover:opacity-100",
      },
      {
        batch: true,
        state: "active",
        className: "border-solid hover:border-solid",
      },
      {
        browseCurrent: true,
        state: "inactive",
        className:
          "disabled:opacity-[0.42] disabled:[filter:grayscale(0.75)] hover:border-[var(--border)] hover:bg-[var(--bg-elev-2)] hover:opacity-[0.42] hover:[filter:grayscale(0.75)]",
      },
      {
        browseCurrent: true,
        state: "partial",
        className:
          "disabled:opacity-[0.88] hover:border-[var(--accent)] hover:bg-[rgba(94,155,255,0.08)] hover:opacity-[0.88]",
      },
      {
        browseCurrent: true,
        state: "active",
        className:
          "hover:border-[var(--ok)] hover:bg-[rgba(78,201,168,0.14)] hover:shadow-[0_0_0_1px_rgba(78,201,168,0.35)]",
      },
    ],
    defaultVariants: {
      state: "inactive",
      batch: false,
      unsupported: false,
      browseCurrent: false,
    },
  },
);

type PlatButtonProps = ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof platButtonVariants>;

export function PlatButton({
  className,
  state,
  batch,
  unsupported,
  browseCurrent,
  ...props
}: PlatButtonProps) {
  return (
    <button
      type="button"
      data-slot="plat-btn"
      className={cn(
        platButtonVariants({ state, batch, unsupported, browseCurrent }),
        batch && "plat-btn-batch",
        className,
      )}
      {...props}
    />
  );
}

export function PlatButtonSpacer({ className }: { className?: string }) {
  return (
    <span
      className={cn("plat-btn plat-btn-spacer invisible pointer-events-none", className)}
      aria-hidden
    />
  );
}
