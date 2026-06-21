import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useAssetOperations } from "./useAssetOperations";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

const mocks = vi.hoisted(() => ({
  saveAsset: vi.fn(() => Promise.resolve("mcp `ai-config` 已保存")),
}));

vi.mock("../api/tauriAssets", () => ({
  addSkillFromRemote: vi.fn(),
  deployAsset: vi.fn(),
  deployAssetFromPlatform: vi.fn(),
  importAsset: vi.fn(),
  retractAsset: vi.fn(),
  revealPath: vi.fn(),
  saveAsset: mocks.saveAsset,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function makeBrowser(overrides: Record<string, unknown> = {}) {
  return {
    activeProject: "user-global",
    activePlatform: "aiconfig",
    activeKind: "mcp" as const,
    browsingSource: true,
    selectedEntries: [],
    issueMap: new Map(),
    browsePath: "/home/.ai-config/mcp.json",
    refreshView: vi.fn(() => Promise.resolve()),
    clearSelection: vi.fn(),
    ...overrides,
  };
}

function makeDrawer() {
  return { runDeleteEntries: vi.fn() };
}

describe("useAssetOperations MCP add", () => {
  afterEach(() => {
    mocks.saveAsset.mockClear();
  });

  it("submitAddMcp saves via saveAsset", async () => {
    const showToast = vi.fn();
    const { result } = renderHook(() =>
      useAssetOperations({
        browser: makeBrowser() as never,
        drawer: makeDrawer() as never,
        showToast,
        requestConfirm: vi.fn(),
        dismissConfirm: vi.fn(),
        setBusy: vi.fn(),
      }),
    );

    act(() => {
      result.current.openAddMcp();
    });
    expect(result.current.addMcpOpen).toBe(true);

    const configJson = JSON.stringify(
      { command: "ai-config", args: ["serve"] },
      null,
      2,
    );
    await act(async () => {
      await result.current.submitAddMcp("ai-config", configJson);
    });

    expect(mocks.saveAsset).toHaveBeenCalledWith(
      "mcp",
      "ai-config",
      configJson,
      "user-global",
    );
    expect(showToast).toHaveBeenCalledWith("ok", "mcp `ai-config` 已保存");
    expect(result.current.addMcpOpen).toBe(false);
  });

  it("openAddMcp ignores non-mcp source context", () => {
    const { result } = renderHook(() =>
      useAssetOperations({
        browser: makeBrowser({
          activeKind: "skill",
          browsingSource: false,
        }) as never,
        drawer: makeDrawer() as never,
        showToast: vi.fn(),
        requestConfirm: vi.fn(),
        dismissConfirm: vi.fn(),
        setBusy: vi.fn(),
      }),
    );

    act(() => {
      result.current.openAddMcp();
    });
    expect(result.current.addMcpOpen).toBe(false);
  });
});
