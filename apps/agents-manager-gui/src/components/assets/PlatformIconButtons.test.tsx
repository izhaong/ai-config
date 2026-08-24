import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { LinkState, Platform } from "../../types";
import { PlatformIconButtons } from "./PlatformIconButtons";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, string>) =>
      opts ? `${key}:${JSON.stringify(opts)}` : key,
  }),
}));

vi.mock("../../platformIcons", () => ({
  PLATFORM_FAVICON: {
    agentsmanager: "/agentsmanager.png",
    cursor: "/cursor.png",
    codex: "/codex.png",
    claude: "/claude.png",
    hermes: "/hermes.png",
  },
  PLATFORM_NAME: {
    agentsmanager: "agents-manager",
    cursor: "Cursor",
    codex: "Codex",
    claude: "Claude",
    hermes: "Hermes",
  },
}));

vi.mock("../../i18n/labels", () => ({
  platformUiLabel: (_t: unknown, active: boolean) =>
    active ? "platform.linked" : "platform.unlinked",
}));

afterEach(() => {
  cleanup();
});

function states(
  overrides: Partial<Record<Platform, LinkState>> = {},
): Record<Platform, LinkState> {
  return {
    agentsmanager: "unlinked",
    cursor: "synced",
    codex: "unlinked",
    claude: "unlinked",
    hermes: "unlinked",
    ...overrides,
  };
}

describe("PlatformIconButtons", () => {
  it("synced 状态显示为激活（绿色边框）", () => {
    render(
      <PlatformIconButtons
        loading={false}
        activePlatform="codex"
        linkStateFor={(plat) => states()[plat]}
        onPlatformClick={vi.fn()}
      />,
    );

    const cursorImg = screen.getByAltText("Cursor");
    const cursorBtn = cursorImg.closest("button")!;
    expect(cursorBtn.className).toContain("border-[var(--ok)]");
    expect(cursorBtn.className).not.toContain("border-dashed");
  });

  it("unlinked 状态为虚线低饱和（border-dashed + opacity 降低）", () => {
    render(
      <PlatformIconButtons
        loading={false}
        activePlatform="codex"
        linkStateFor={(plat) => states({ cursor: "unlinked" })[plat]}
        onPlatformClick={vi.fn()}
      />,
    );

    const cursorImg = screen.getByAltText("Cursor");
    const cursorBtn = cursorImg.closest("button")!;
    expect(cursorBtn.className).toContain("border-dashed");
    expect(cursorBtn.className).not.toContain("border-[var(--ok)]");
    expect(cursorBtn.className).toContain("opacity-[0.3]");
  });

  it("当前浏览平台 icon 不可点击，其它平台可点击", () => {
    const onPlatformClick = vi.fn();
    render(
      <PlatformIconButtons
        loading={false}
        activePlatform="cursor"
        linkStateFor={(plat) => states()[plat]}
        onPlatformClick={onPlatformClick}
      />,
    );

    const buttons = screen.getAllByRole("button");
    const cursorBtn = buttons.find((b) => b.getAttribute("title")?.includes("Cursor"))!;
    const codexBtn = buttons.find((b) => b.getAttribute("title")?.includes("Codex"))!;

    expect((cursorBtn as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(cursorBtn);
    expect(onPlatformClick).not.toHaveBeenCalled();

    expect((codexBtn as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(codexBtn);
    expect(onPlatformClick).toHaveBeenCalledWith("codex");
  });

  it("当前浏览平台 icon 使用 browseCurrent 样式类", () => {
    render(
      <PlatformIconButtons
        loading={false}
        activePlatform="cursor"
        linkStateFor={(plat) => states()[plat]}
        onPlatformClick={vi.fn()}
      />,
    );

    const cursorImg = screen.getByAltText("Cursor");
    const cursorBtn = cursorImg.closest("button")!;
    expect(cursorBtn.className).toContain("cursor-default");
  });
});
