import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { HookLifecycleView } from "../../types";
import { HookLifecycleButtons } from "./HookLifecycleButtons";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, string>) => {
      if (key.startsWith("hookLifecycle.chips.")) {
        return key.split(".").pop() ?? key;
      }
      if (opts) {
        return `${key}:${JSON.stringify(opts)}`;
      }
      return key;
    },
  }),
}));

function view(
  lifecycle: string,
  overrides: Partial<HookLifecycleView> = {},
): HookLifecycleView {
  return {
    lifecycle,
    group: "agent",
    label: lifecycle,
    short_label: lifecycle,
    active: false,
    supported: true,
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
});

describe("HookLifecycleButtons", () => {
  it("toggles one side of a split pair", () => {
    const onTogglePair = vi.fn();
    render(
      <HookLifecycleButtons
        loading={false}
        lifecycles={[
          view("sessionStart", { active: true }),
          view("sessionEnd"),
          view("stop"),
        ]}
        onTogglePair={onTogglePair}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /sessionEnd/ }));
    expect(onTogglePair).toHaveBeenCalledWith([
      { lifecycle: "sessionEnd", enabled: true },
    ]);
  });

  it("hides pairs unsupported on current platform", () => {
    render(
      <HookLifecycleButtons
        loading={false}
        lifecycles={[
          view("sessionStart", { supported: false, supported_platforms: ["cursor"] }),
          view("sessionEnd", { supported: false, supported_platforms: ["cursor"] }),
          view("preToolUse", { supported: true, supported_platforms: ["cursor", "codex"] }),
          view("postToolUse", { supported: true, supported_platforms: ["cursor", "codex"] }),
        ]}
        onTogglePair={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: /sessionStart/ })).toBeNull();
    expect(screen.getByRole("button", { name: /preToolUse/ })).toBeTruthy();
  });
});
