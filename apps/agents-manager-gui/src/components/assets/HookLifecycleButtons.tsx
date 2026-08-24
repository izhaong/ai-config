import { Bot, Layers, Terminal } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { buildHookLifecyclePairs } from "../../utils/hookLifecyclePairs";
import type { HookLifecycleGroup, HookLifecycleView } from "../../types";
import { LifecycleChip } from "./LifecycleChip";
import { LifecycleSplitCapsule } from "./LifecycleSplitCapsule";

interface HookLifecycleButtonsProps {
  loading: boolean;
  lifecycles: HookLifecycleView[];
  onTogglePair: (lifecycles: Array<{ lifecycle: string; enabled: boolean }>) => void;
}

const GROUP_ORDER: HookLifecycleGroup[] = ["agent", "tab", "workspace"];

const GROUP_ICON: Record<
  HookLifecycleGroup,
  React.ComponentType<{ size?: number; className?: string }>
> = {
  agent: Bot,
  tab: Terminal,
  workspace: Layers,
};

function groupLabel(t: (k: string) => string, group: HookLifecycleGroup): string {
  switch (group) {
    case "agent":
      return t("hookLifecycle.groupAgent");
    case "tab":
      return t("hookLifecycle.groupTab");
    case "workspace":
      return t("hookLifecycle.groupWorkspace");
  }
}

function lifecycleLabel(
  t: (k: string) => string,
  member: HookLifecycleView,
): string {
  const chipKey = `hookLifecycle.chips.${member.lifecycle}`;
  const translated = t(chipKey);
  if (translated !== chipKey) {
    return translated;
  }
  return member.short_label || member.label;
}

function lifecycleTitle(
  t: (k: string, opts?: Record<string, string>) => string,
  member: HookLifecycleView,
): string {
  const label = lifecycleLabel(t, member);
  if (!member.supported) {
    const platforms = member.supported_platforms?.join(", ") ?? "";
    const hint = platforms
      ? t("hookLifecycle.unsupportedOnPlatformWithList", {
          lifecycle: label,
          platforms,
        })
      : t("hookLifecycle.unsupported", { lifecycle: label });
    return hint;
  }
  return member.active
    ? t("hookLifecycle.clickDisable", { lifecycle: label })
    : t("hookLifecycle.clickEnable", { lifecycle: label });
}

export function HookLifecycleButtons({
  loading,
  lifecycles,
  onTogglePair,
}: HookLifecycleButtonsProps) {
  const { t } = useTranslation();

  const pairs = useMemo(() => buildHookLifecyclePairs(lifecycles), [lifecycles]);

  const byGroup = useMemo(
    () =>
      GROUP_ORDER.map((group) => ({
        group,
        items: pairs.filter((item) => item.group === group),
      })).filter((g) => g.items.length > 0),
    [pairs],
  );

  if (byGroup.length === 0) {
    return null;
  }

  const activeCount = lifecycles.filter((item) => item.active).length;
  const supportedCount = lifecycles.filter((item) => item.supported).length;

  const toggleOne = (lifecycle: string, enabled: boolean) => {
    onTogglePair([{ lifecycle, enabled }]);
  };

  return (
    <div
      className={cn(
        "hook-lifecycle-panel mt-2.5 w-full min-w-0 rounded-lg",
        "border border-[var(--border)]/80 bg-[var(--bg-elev-1)]/90",
        "px-2.5 py-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]",
      )}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
    >
      <div className="mb-2 flex min-w-0 items-center justify-between gap-2">
        <span className="text-[10px] font-semibold uppercase tracking-[0.06em] text-[var(--fg-dim)]">
          {t("hookLifecycle.panelTitle")}
        </span>
        <span
          className={cn(
            "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium tabular-nums",
            activeCount > 0
              ? "bg-[rgba(78,201,168,0.14)] text-[var(--ok)]"
              : "bg-[var(--bg-elev-2)] text-[var(--fg-dim)]",
          )}
        >
          {t("hookLifecycle.enabledBadge", {
            active: String(activeCount),
            total: String(supportedCount),
          })}
        </span>
      </div>

      <div className="flex flex-col gap-2.5">
        {byGroup.map(({ group, items }) => {
          const Icon = GROUP_ICON[group];
          const groupMembers = items.flatMap((pair) => pair.members);
          const groupActive = groupMembers.filter((m) => m.active && m.supported).length;
          const groupSupported = groupMembers.filter((m) => m.supported).length;

          return (
            <section key={group} className="min-w-0">
              <div className="mb-1.5 flex items-center gap-1.5 text-[10px] text-[var(--fg-dim)]">
                <Icon size={11} className="shrink-0 opacity-70" aria-hidden />
                <span className="font-medium">{groupLabel(t, group)}</span>
                <span className="text-[var(--fg-dim)]/70">·</span>
                <span className="tabular-nums text-[var(--fg-dim)]/80">
                  {groupActive}/{groupSupported}
                </span>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {items
                  .filter((pair) =>
                    pair.members.some((m) => m.supported || m.active),
                  )
                  .map((pair) => {
                  if (pair.members.length >= 2) {
                    return (
                      <LifecycleSplitCapsule
                        key={pair.id}
                        members={pair.members}
                        loading={loading}
                        segmentLabel={(member) => lifecycleLabel(t, member)}
                        segmentTitle={(member) => lifecycleTitle(t, member)}
                        onToggle={toggleOne}
                      />
                    );
                  }

                  const member = pair.members[0]!;
                  const title = lifecycleTitle(t, member);
                  const state = !member.supported
                    ? "unsupported"
                    : member.active
                      ? "active"
                      : "inactive";

                  return (
                    <LifecycleChip
                      key={pair.id}
                      state={state}
                      disabled={loading || !member.supported}
                      className="h-7 px-2.5 text-[11px]"
                      title={title}
                      aria-pressed={member.active}
                      aria-label={title}
                      onClick={() => {
                        if (loading || !member.supported) return;
                        toggleOne(member.lifecycle, !member.active);
                      }}
                    >
                      {lifecycleLabel(t, member)}
                    </LifecycleChip>
                  );
                })}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}
