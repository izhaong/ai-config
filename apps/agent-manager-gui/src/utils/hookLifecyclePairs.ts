import type { HookLifecycleGroup, HookLifecycleView } from "../types";

/** Cursor 生命周期对/组：一前一后合并为一颗胶囊。 */
export interface HookLifecyclePairDef {
  id: string;
  group: HookLifecycleGroup;
  lifecycles: readonly string[];
}

export const HOOK_LIFECYCLE_PAIRS: readonly HookLifecyclePairDef[] = [
  { id: "session", group: "agent", lifecycles: ["sessionStart", "sessionEnd"] },
  { id: "tool", group: "agent", lifecycles: ["preToolUse", "postToolUse"] },
  { id: "toolFail", group: "agent", lifecycles: ["postToolUseFailure"] },
  {
    id: "subagent",
    group: "agent",
    lifecycles: ["subagentStart", "subagentStop"],
  },
  {
    id: "shell",
    group: "agent",
    lifecycles: ["beforeShellExecution", "afterShellExecution"],
  },
  {
    id: "mcp",
    group: "agent",
    lifecycles: ["beforeMCPExecution", "afterMCPExecution"],
  },
  {
    id: "file",
    group: "agent",
    lifecycles: ["beforeReadFile", "afterFileEdit"],
  },
  { id: "prompt", group: "agent", lifecycles: ["beforeSubmitPrompt"] },
  { id: "compact", group: "agent", lifecycles: ["preCompact"] },
  { id: "stop", group: "agent", lifecycles: ["stop"] },
  {
    id: "agentOut",
    group: "agent",
    lifecycles: ["afterAgentResponse", "afterAgentThought"],
  },
  {
    id: "tab",
    group: "tab",
    lifecycles: ["beforeTabFileRead", "afterTabFileEdit"],
  },
  { id: "workspace", group: "workspace", lifecycles: ["workspaceOpen"] },
] as const;

export type HookLifecyclePairState =
  | "active"
  | "partial"
  | "inactive"
  | "unsupported";

export interface HookLifecyclePairView {
  id: string;
  group: HookLifecycleGroup;
  lifecycles: string[];
  /** 当前平台可操作的成员 */
  supportedLifecycles: string[];
  state: HookLifecyclePairState;
  members: HookLifecycleView[];
}

function lookup(
  map: Map<string, HookLifecycleView>,
  id: string,
): HookLifecycleView | undefined {
  return map.get(id);
}

export function buildHookLifecyclePairs(
  lifecycles: HookLifecycleView[],
): HookLifecyclePairView[] {
  const map = new Map(lifecycles.map((item) => [item.lifecycle, item]));

  return HOOK_LIFECYCLE_PAIRS.map((def) => {
    const members = def.lifecycles
      .map((id) => lookup(map, id))
      .filter((item): item is HookLifecycleView => item != null);

    if (members.length === 0) {
      return null;
    }

    const supported = members.filter((m) => m.supported);
    const supportedLifecycles = supported.map((m) => m.lifecycle);
    const activeSupported = supported.filter((m) => m.active);

    let state: HookLifecyclePairState;
    if (supported.length === 0) {
      state = "unsupported";
    } else if (activeSupported.length === 0) {
      state = "inactive";
    } else if (activeSupported.length === supported.length) {
      state = "active";
    } else {
      state = "partial";
    }

    return {
      id: def.id,
      group: def.group,
      lifecycles: [...def.lifecycles],
      supportedLifecycles,
      state,
      members,
    };
  }).filter((item): item is HookLifecyclePairView => item != null);
}

/** 点击胶囊：全开 → 全关；半开/未开 → 全开。 */
export function pairTogglePlan(pair: HookLifecyclePairView): Array<{
  lifecycle: string;
  enabled: boolean;
}> {
  const { supportedLifecycles, members, state } = pair;
  if (supportedLifecycles.length === 0) {
    return [];
  }

  const memberMap = new Map(members.map((m) => [m.lifecycle, m]));

  if (state === "active") {
    return supportedLifecycles
      .filter((id) => memberMap.get(id)?.active)
      .map((lifecycle) => ({ lifecycle, enabled: false }));
  }

  return supportedLifecycles
    .filter((id) => !memberMap.get(id)?.active)
    .map((lifecycle) => ({ lifecycle, enabled: true }));
}
