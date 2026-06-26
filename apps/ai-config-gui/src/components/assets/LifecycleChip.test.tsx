import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { LifecycleChip } from "./LifecycleChip";

afterEach(() => {
  cleanup();
});

describe("LifecycleChip", () => {
  it("renders label only without leading status dot", () => {
    const { container } = render(
      <LifecycleChip state="active" aria-label="完成">
        完成
      </LifecycleChip>,
    );

    expect(screen.getByRole("button", { name: "完成" }).textContent).toBe("完成");
    expect(container.querySelector('[aria-hidden][class*="size-1.5"]')).toBeNull();
    expect(screen.getByRole("button").className).toMatch(/text-\[var\(--ok\)\]/);
  });
});
