import { cn } from "@/lib/utils";

import type { HookLifecycleView } from "../../types";

interface LifecycleSplitCapsuleProps {
  members: HookLifecycleView[];
  loading: boolean;
  segmentLabel: (member: HookLifecycleView) => string;
  segmentTitle: (member: HookLifecycleView) => string;
  onToggle: (lifecycle: string, enabled: boolean) => void;
}

function segmentStateClass(member: HookLifecycleView): string {
  if (!member.supported) {
    return "cursor-not-allowed text-[var(--fg-dim)] opacity-35";
  }
  if (member.active) {
    return [
      "bg-[rgba(78,201,168,0.2)] font-semibold text-[var(--ok)]",
      "hover:bg-[rgba(229,116,116,0.18)] hover:text-[var(--err)]",
    ].join(" ");
  }
  return [
    "bg-transparent font-medium text-[var(--fg-dim)]",
    "hover:bg-[rgba(94,155,255,0.12)] hover:text-[var(--fg)]",
  ].join(" ");
}

export function LifecycleSplitCapsule({
  members,
  loading,
  segmentLabel,
  segmentTitle,
  onToggle,
}: LifecycleSplitCapsuleProps) {
  const supported = members.filter((m) => m.supported);
  const activeCount = supported.filter((m) => m.active).length;
  const allUnsupported = supported.length === 0;

  const shellBorder = allUnsupported
    ? "border-[var(--border)]/35 opacity-[0.35]"
    : activeCount === supported.length && activeCount > 0
      ? "border-[rgba(78,201,168,0.55)]"
      : activeCount > 0
        ? "border-[rgba(94,155,255,0.45)]"
        : "border-[var(--border)]";

  return (
    <div
      data-slot="lifecycle-split-capsule"
      className={cn(
        "hook-lifecycle-split inline-flex h-7 max-w-full shrink-0 overflow-hidden rounded-full border bg-[var(--bg-elev-2)]",
        shellBorder,
      )}
    >
      {members.map((member, index) => {
        const title = segmentTitle(member);
        const isLast = index === members.length - 1;

        return (
          <button
            key={member.lifecycle}
            type="button"
            data-slot="lifecycle-segment"
            data-active={member.active && member.supported ? "true" : "false"}
            disabled={loading || !member.supported}
            title={title}
            aria-label={title}
            aria-pressed={member.active}
            className={cn(
              "inline-flex h-full min-w-0 items-center rounded-none border-0 px-2.5 text-[10px] leading-none",
              "transition-[background-color,color,font-weight] duration-100",
              "focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--accent)]/45",
              !isLast && "border-r border-[var(--border)]",
              segmentStateClass(member),
            )}
            onClick={() => {
              if (loading || !member.supported) return;
              onToggle(member.lifecycle, !member.active);
            }}
          >
            <span className="truncate">{segmentLabel(member)}</span>
          </button>
        );
      })}
    </div>
  );
}
