import { describe, expect, it } from "vitest";

import type { HookLifecycleView } from "../types";
import { buildHookLifecyclePairs, pairTogglePlan } from "./hookLifecyclePairs";

function view(
  lifecycle: string,
  active: boolean,
  supported = true,
): HookLifecycleView {
  return {
    lifecycle,
    group: "agent",
    label: lifecycle,
    short_label: lifecycle,
    active,
    supported,
  };
}

describe("hookLifecyclePairs", () => {
  it("merges session start/end into one pair", () => {
    const pairs = buildHookLifecyclePairs([
      view("sessionStart", true),
      view("sessionEnd", false),
    ]);
    const session = pairs.find((p) => p.id === "session");
    expect(session?.state).toBe("partial");
    expect(pairTogglePlan(session!).map((x) => x.lifecycle)).toEqual([
      "sessionEnd",
    ]);
  });

  it("disables both when pair fully active", () => {
    const pairs = buildHookLifecyclePairs([
      view("beforeShellExecution", true),
      view("afterShellExecution", true),
    ]);
    const shell = pairs.find((p) => p.id === "shell");
    expect(shell?.state).toBe("active");
    expect(pairTogglePlan(shell!)).toEqual([
      { lifecycle: "beforeShellExecution", enabled: false },
      { lifecycle: "afterShellExecution", enabled: false },
    ]);
  });
});
