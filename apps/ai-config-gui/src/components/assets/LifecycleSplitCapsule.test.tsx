import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { HookLifecycleView } from "../../types";
import { LifecycleSplitCapsule } from "./LifecycleSplitCapsule";

const members: HookLifecycleView[] = [
  {
    lifecycle: "beforeShellExecution",
    group: "agent",
    label: "beforeShellExecution",
    short_label: "Shell前",
    active: true,
    supported: true,
  },
  {
    lifecycle: "afterShellExecution",
    group: "agent",
    label: "afterShellExecution",
    short_label: "Shell后",
    active: false,
    supported: true,
  },
];

afterEach(() => {
  cleanup();
});

describe("LifecycleSplitCapsule", () => {
  it("renders split capsule without status dots", () => {
    const onToggle = vi.fn();
    const { container } = render(
      <LifecycleSplitCapsule
        members={members}
        loading={false}
        segmentLabel={(m) => m.short_label}
        segmentTitle={(m) => m.lifecycle}
        onToggle={onToggle}
      />,
    );

    expect(container.querySelector('[aria-hidden][class*="rounded-full"]')).toBeNull();

    const buttons = screen.getAllByRole("button");
    expect(buttons).toHaveLength(2);
    expect(buttons[0]?.getAttribute("data-active")).toBe("true");
    expect(buttons[0]?.className).toMatch(/rounded-none/);
    expect(buttons[0]?.className).toMatch(/text-\[var\(--ok\)\]/);

    const root = container.querySelector("[data-slot='lifecycle-split-capsule']");
    expect(root?.className).toMatch(/rounded-full/);
  });

  it("toggles each segment independently", () => {
    const onToggle = vi.fn();
    render(
      <LifecycleSplitCapsule
        members={members}
        loading={false}
        segmentLabel={(m) => m.short_label}
        segmentTitle={(m) => m.lifecycle}
        onToggle={onToggle}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "afterShellExecution" }));
    expect(onToggle).toHaveBeenCalledWith("afterShellExecution", true);
  });
});
