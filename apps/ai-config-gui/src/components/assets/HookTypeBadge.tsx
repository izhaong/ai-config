import { cn } from "@/lib/utils";

interface HookTypeBadgeProps {
  hookType: "command" | "prompt";
  className?: string;
}

export function HookTypeBadge({ hookType, className }: HookTypeBadgeProps) {
  return (
    <span
      className={cn(
        "hook-type-badge inline-flex shrink-0 items-center rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
        hookType === "prompt"
          ? "bg-violet-500/15 text-violet-700 dark:text-violet-300"
          : "bg-sky-500/15 text-sky-700 dark:text-sky-300",
        className,
      )}
    >
      {hookType}
    </span>
  );
}
